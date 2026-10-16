//! Archived tabs (`archive.json`): auto-closed idle tabs kept restorable.

use std::path::Path;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::paths;

pub const ARCHIVE_FILE: &str = "archive.json";
/// Oldest entries beyond this are dropped.
pub const MAX_ARCHIVE_ENTRIES: usize = 300;

const DAY: i64 = 86_400;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArchiveEntry {
    pub id: Uuid,
    pub url: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub favicon_url: Option<String>,
    /// UNIX seconds.
    pub archived_at: f64,
}

/// Library grouping of archive entries by local calendar date.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ArchiveGroup {
    Today,
    Yesterday,
    /// Earlier in the current (Monday-based) week.
    ThisWeek,
    Older,
}

impl ArchiveGroup {
    pub const ALL: [ArchiveGroup; 4] = [
        ArchiveGroup::Today,
        ArchiveGroup::Yesterday,
        ArchiveGroup::ThisWeek,
        ArchiveGroup::Older,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ArchiveGroup::Today => "Today",
            ArchiveGroup::Yesterday => "Yesterday",
            ArchiveGroup::ThisWeek => "This Week",
            ArchiveGroup::Older => "Older",
        }
    }

    /// Classify `time` relative to `now`, both UNIX seconds, in a timezone
    /// `utc_offset_secs` east of UTC.
    pub fn classify(time: f64, now: f64, utc_offset_secs: i64) -> ArchiveGroup {
        let day_of = |t: f64| ((t.floor() as i64) + utc_offset_secs).div_euclid(DAY);
        let today = day_of(now);
        let day = day_of(time);
        // 1970-01-01 was a Thursday; Monday = 0.
        let week_start = today - (today + 3).rem_euclid(7);
        if day == today {
            ArchiveGroup::Today
        } else if day == today - 1 {
            ArchiveGroup::Yesterday
        } else if (week_start..week_start + 7).contains(&day) {
            ArchiveGroup::ThisWeek
        } else {
            ArchiveGroup::Older
        }
    }
}

/// Persistent archive, newest first.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ArchiveStore {
    pub entries: Vec<ArchiveEntry>,
}

impl ArchiveStore {
    pub fn load(dir: &Path) -> ArchiveStore {
        paths::read_json(&dir.join(ARCHIVE_FILE)).unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) -> anyhow::Result<()> {
        paths::write_json_atomic(&dir.join(ARCHIVE_FILE), self)
    }

    /// Archive a page now. Blank and `mori://` pages are skipped; an existing
    /// entry for the same URL is replaced so the newest sits on top. Returns
    /// whether an entry was added.
    pub fn add(&mut self, url: &str, title: &str, favicon_url: Option<String>) -> bool {
        self.add_at(url, title, favicon_url, paths::now())
    }

    /// [`ArchiveStore::add`] with an explicit timestamp.
    pub fn add_at(
        &mut self,
        url: &str,
        title: &str,
        favicon_url: Option<String>,
        time: f64,
    ) -> bool {
        if url.is_empty() || url == "about:blank" || url.starts_with("mori://") {
            return false;
        }
        self.entries.retain(|e| e.url != url);
        self.entries.insert(
            0,
            ArchiveEntry {
                id: Uuid::new_v4(),
                url: url.to_string(),
                title: if title.is_empty() {
                    url.to_string()
                } else {
                    title.to_string()
                },
                favicon_url,
                archived_at: time,
            },
        );
        self.entries.truncate(MAX_ARCHIVE_ENTRIES);
        true
    }

    /// Remove and return an entry (e.g. when it is reopened).
    pub fn remove(&mut self, id: Uuid) -> Option<ArchiveEntry> {
        let idx = self.entries.iter().position(|e| e.id == id)?;
        Some(self.entries.remove(idx))
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn list(&self) -> &[ArchiveEntry] {
        &self.entries
    }

    /// Entries bucketed into Today / Yesterday / This Week / Older (in that
    /// order, empty groups omitted), keeping list order within each group.
    pub fn grouped(
        &self,
        now: f64,
        utc_offset_secs: i64,
    ) -> Vec<(ArchiveGroup, Vec<&ArchiveEntry>)> {
        ArchiveGroup::ALL
            .iter()
            .filter_map(|&group| {
                let items: Vec<&ArchiveEntry> = self
                    .entries
                    .iter()
                    .filter(|e| {
                        ArchiveGroup::classify(e.archived_at, now, utc_offset_secs) == group
                    })
                    .collect();
                (!items.is_empty()).then_some((group, items))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::test_support::TempDir;

    // 2026-10-16 is a Friday. 12:00 UTC.
    const NOW: f64 = 1_792_108_800.0 + 12.0 * 3600.0;

    #[test]
    fn now_constant_is_friday_noon() {
        let day = (NOW as i64).div_euclid(DAY);
        assert_eq!((day + 3).rem_euclid(7), 4); // Monday=0 → Friday=4
    }

    #[test]
    fn classify_groups() {
        let h = 3600.0;
        assert_eq!(ArchiveGroup::classify(NOW - h, NOW, 0), ArchiveGroup::Today);
        assert_eq!(
            ArchiveGroup::classify(NOW - 24.0 * h, NOW, 0),
            ArchiveGroup::Yesterday
        );
        assert_eq!(
            ArchiveGroup::classify(NOW - 4.0 * 24.0 * h, NOW, 0),
            ArchiveGroup::ThisWeek
        ); // Monday
        assert_eq!(
            ArchiveGroup::classify(NOW - 5.0 * 24.0 * h, NOW, 0),
            ArchiveGroup::Older
        ); // Sunday
        // Thu 23:00 UTC is yesterday in UTC but already Friday at UTC+2.
        assert_eq!(
            ArchiveGroup::classify(NOW - 13.0 * h, NOW, 0),
            ArchiveGroup::Yesterday
        );
        assert_eq!(
            ArchiveGroup::classify(NOW - 13.0 * h, NOW, 2 * 3600),
            ArchiveGroup::Today
        );
    }

    #[test]
    fn add_dedupe_group() {
        let tmp = TempDir::new();
        let mut a = ArchiveStore::default();
        assert!(!a.add("about:blank", "", None));
        assert!(!a.add("mori://newtab", "", None));
        assert!(a.add_at("https://old.com/", "Old", None, NOW - 30.0 * 86400.0));
        assert!(a.add_at("https://a.com/", "", None, NOW - 86400.0));
        assert!(a.add_at(
            "https://a.com/",
            "A",
            Some("https://a.com/f.ico".into()),
            NOW - 60.0
        ));
        assert_eq!(a.list().len(), 2);
        assert_eq!(a.list()[0].title, "A");
        let groups = a.grouped(NOW, 0);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].0, ArchiveGroup::Today);
        assert_eq!(groups[1].0, ArchiveGroup::Older);
        assert_eq!(groups[1].0.label(), "Older");
        a.save(tmp.path()).unwrap();
        assert_eq!(ArchiveStore::load(tmp.path()), a);
        let id = a.list()[0].id;
        assert_eq!(a.remove(id).map(|e| e.url), Some("https://a.com/".into()));
    }

    #[test]
    fn cap() {
        let mut a = ArchiveStore::default();
        for i in 0..(MAX_ARCHIVE_ENTRIES + 3) {
            a.add_at(&format!("https://e{i}.com/"), "", None, i as f64);
        }
        assert_eq!(a.list().len(), MAX_ARCHIVE_ENTRIES);
    }
}
