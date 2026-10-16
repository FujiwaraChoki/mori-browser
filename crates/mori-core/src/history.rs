//! Browsing history (`history.json`), newest first.

use std::path::Path;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::paths;

pub const HISTORY_FILE: &str = "history.json";
/// Oldest entries beyond this are dropped.
pub const MAX_HISTORY_ENTRIES: usize = 2000;

/// URL prefixes that are never recorded (matched case-insensitively).
const UNRECORDED_PREFIXES: [&str; 6] = [
    "about:",
    "chrome:",
    "devtools:",
    "data:",
    "mori:",
    "view-source:",
];

/// One visited URL, collapsed across visits.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: Uuid,
    pub url: String,
    #[serde(default)]
    pub title: String,
    /// UNIX seconds of the latest visit.
    pub last_visited: f64,
    #[serde(default = "one")]
    pub visit_count: u32,
}

fn one() -> u32 {
    1
}

impl HistoryEntry {
    /// The title, or the URL when the page had none.
    pub fn display_title(&self) -> &str {
        if self.title.trim().is_empty() {
            &self.url
        } else {
            &self.title
        }
    }
}

/// Persistent history; `entries` is ordered most recent first.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct HistoryStore {
    pub entries: Vec<HistoryEntry>,
}

/// True when visits to `url` belong in history.
pub fn is_recordable(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    !lower.is_empty() && !UNRECORDED_PREFIXES.iter().any(|p| lower.starts_with(p))
}

impl HistoryStore {
    pub fn load(dir: &Path) -> HistoryStore {
        paths::read_json(&dir.join(HISTORY_FILE)).unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) -> anyhow::Result<()> {
        paths::write_json_atomic(&dir.join(HISTORY_FILE), self)
    }

    /// Record a visit at the current time. See [`HistoryStore::record_at`].
    pub fn record(&mut self, url: &str, title: &str) -> bool {
        self.record_at(url, title, paths::now())
    }

    /// Record a visit at `time`. A revisit moves the entry to the top, bumps
    /// its count and (if non-empty) updates its title. Returns whether
    /// anything changed (internal pages are ignored).
    pub fn record_at(&mut self, url: &str, title: &str, time: f64) -> bool {
        if !is_recordable(url) {
            return false;
        }
        if let Some(idx) = self.entries.iter().position(|e| e.url == url) {
            let mut entry = self.entries.remove(idx);
            entry.last_visited = time;
            entry.visit_count = entry.visit_count.saturating_add(1);
            if !title.is_empty() {
                entry.title = title.to_string();
            }
            self.entries.insert(0, entry);
        } else {
            self.entries.insert(
                0,
                HistoryEntry {
                    id: Uuid::new_v4(),
                    url: url.to_string(),
                    title: title.to_string(),
                    last_visited: time,
                    visit_count: 1,
                },
            );
            self.entries.truncate(MAX_HISTORY_ENTRIES);
        }
        true
    }

    /// Set the title of the entry for `url` (titles arrive after commit).
    pub fn update_title(&mut self, url: &str, title: &str) -> bool {
        if title.is_empty() {
            return false;
        }
        match self.entries.iter_mut().find(|e| e.url == url) {
            Some(entry) if entry.title != title => {
                entry.title = title.to_string();
                true
            }
            _ => false,
        }
    }

    /// The `n` most recent entries.
    pub fn recent(&self, n: usize) -> &[HistoryEntry] {
        &self.entries[..n.min(self.entries.len())]
    }

    /// Case-insensitive substring match on URL or title, most-visited first,
    /// then most recent. An empty query matches nothing.
    pub fn search(&self, query: &str, limit: usize) -> Vec<&HistoryEntry> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return Vec::new();
        }
        let mut hits: Vec<&HistoryEntry> = self
            .entries
            .iter()
            .filter(|e| e.url.to_lowercase().contains(&q) || e.title.to_lowercase().contains(&q))
            .collect();
        hits.sort_by(|a, b| {
            b.visit_count
                .cmp(&a.visit_count)
                .then(b.last_visited.total_cmp(&a.last_visited))
        });
        hits.truncate(limit);
        hits
    }

    pub fn remove(&mut self, id: Uuid) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| e.id != id);
        self.entries.len() != before
    }

    /// Remove visits since `since` (UNIX seconds); returns how many went.
    /// An entry's latest visit decides (older visits aren't kept apart).
    pub fn remove_since(&mut self, since: f64) -> usize {
        let before = self.entries.len();
        self.entries.retain(|e| e.last_visited < since);
        before - self.entries.len()
    }

    /// URLs visited since `since`, or ever with `None`.
    pub fn urls_since(&self, since: Option<f64>) -> impl Iterator<Item = &str> {
        self.entries
            .iter()
            .filter(move |e| since.is_none_or(|t| e.last_visited >= t))
            .map(|e| e.url.as_str())
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Undo a [`HistoryStore::clear`]: put `cleared` (the entries as they
    /// were) back underneath anything visited since. A page visited both
    /// before and after keeps one entry with the newer visit on top and the
    /// visit counts added up.
    pub fn restore(&mut self, cleared: HistoryStore) {
        let mut merged = std::mem::take(&mut self.entries);
        for old in cleared.entries {
            match merged.iter_mut().find(|e| e.url == old.url) {
                Some(newer) => {
                    newer.visit_count = newer.visit_count.saturating_add(old.visit_count);
                    if newer.title.is_empty() {
                        newer.title = old.title;
                    }
                }
                None => merged.push(old),
            }
        }
        merged.sort_by(|a, b| b.last_visited.total_cmp(&a.last_visited));
        merged.truncate(MAX_HISTORY_ENTRIES);
        self.entries = merged;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::test_support::TempDir;

    #[test]
    fn ordering_and_revisits() {
        let mut h = HistoryStore::default();
        assert!(h.record_at("https://a.com/", "A", 1.0));
        assert!(h.record_at("https://b.com/", "B", 2.0));
        assert!(h.record_at("https://a.com/", "", 3.0));
        assert_eq!(h.entries.len(), 2);
        assert_eq!(h.entries[0].url, "https://a.com/");
        assert_eq!(h.entries[0].visit_count, 2);
        assert_eq!(h.entries[0].title, "A");
        assert_eq!(h.entries[0].last_visited, 3.0);
        assert_eq!(h.recent(1).len(), 1);
        assert_eq!(h.recent(10).len(), 2);
    }

    #[test]
    fn removes_a_time_range() {
        let mut h = HistoryStore::default();
        h.record_at("https://old.test/", "", 10.0);
        h.record_at("https://new.test/", "", 100.0);
        h.record_at("https://newer.test/", "", 200.0);
        assert_eq!(h.urls_since(Some(100.0)).collect::<Vec<_>>(), ["https://newer.test/", "https://new.test/"]);
        assert_eq!(h.urls_since(None).count(), 3);
        assert_eq!(h.remove_since(100.0), 2);
        assert_eq!(h.entries.len(), 1);
        assert_eq!(h.entries[0].url, "https://old.test/");
    }

    #[test]
    fn skips_internal_pages() {
        let mut h = HistoryStore::default();
        for url in [
            "",
            "about:blank",
            "Chrome://settings",
            "devtools://x",
            "data:text/plain,a",
            "mori://newtab",
            "view-source:https://a.com",
        ] {
            assert!(!h.record(url, "x"), "{url}");
        }
        assert!(h.entries.is_empty());
    }

    #[test]
    fn cap() {
        let mut h = HistoryStore::default();
        for i in 0..(MAX_HISTORY_ENTRIES + 5) {
            h.record_at(&format!("https://e{i}.com/"), "", i as f64);
        }
        assert_eq!(h.entries.len(), MAX_HISTORY_ENTRIES);
        assert_eq!(
            h.entries[0].url,
            format!("https://e{}.com/", MAX_HISTORY_ENTRIES + 4)
        );
    }

    #[test]
    fn search_sorted_by_visits_then_recency() {
        let mut h = HistoryStore::default();
        h.record_at("https://rust-lang.org/", "Rust", 1.0);
        h.record_at("https://docs.rs/", "Docs RS", 2.0);
        h.record_at("https://crates.io/", "crates: Rust packages", 3.0);
        h.record_at("https://rust-lang.org/", "Rust", 4.0);
        let hits = h.search("RUST", 10);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].url, "https://rust-lang.org/");
        assert_eq!(hits[1].url, "https://crates.io/");
        assert!(h.search("", 10).is_empty());
        assert_eq!(h.search("s", 1).len(), 1);
    }

    #[test]
    fn restore_after_clear_merges_with_new_visits() {
        let mut h = HistoryStore::default();
        h.record_at("https://a.com/", "A", 1.0);
        h.record_at("https://b.com/", "B", 2.0);
        h.record_at("https://b.com/", "B", 3.0);
        let snapshot = h.clone();
        h.clear();
        h.record_at("https://c.com/", "C", 4.0);
        h.record_at("https://b.com/", "", 5.0);
        h.restore(snapshot);
        let urls: Vec<&str> = h.entries.iter().map(|e| e.url.as_str()).collect();
        assert_eq!(urls, ["https://b.com/", "https://c.com/", "https://a.com/"]);
        assert_eq!(h.entries[0].visit_count, 3);
        assert_eq!(h.entries[0].title, "B");
        assert_eq!(h.entries[0].last_visited, 5.0);
    }

    #[test]
    fn titles_remove_persist() {
        let tmp = TempDir::new();
        let mut h = HistoryStore::default();
        h.record("https://a.com/", "");
        assert!(h.update_title("https://a.com/", "Alpha"));
        assert_eq!(h.entries[0].display_title(), "Alpha");
        h.save(tmp.path()).unwrap();
        let loaded = HistoryStore::load(tmp.path());
        assert_eq!(loaded, h);
        let id = h.entries[0].id;
        assert!(h.remove(id));
        assert!(!h.remove(id));
        h.record("https://b.com/", "");
        h.clear();
        assert!(h.entries.is_empty());
    }
}
