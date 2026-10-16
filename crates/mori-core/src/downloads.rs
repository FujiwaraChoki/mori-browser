//! The downloads list: what was downloaded, where it went, and how it
//! ended, kept across restarts (`downloads.json`).
//!
//! Live progress comes from the engine; this store merges those updates by
//! the engine's per-session id and keeps the most recent entries.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::paths;

pub const DOWNLOADS_FILE: &str = "downloads.json";
/// Entries kept, newest first.
pub const MAX_ENTRIES: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadState {
    InProgress,
    Paused,
    Complete,
    Canceled,
    Interrupted,
}

impl DownloadState {
    /// Running or paused: more updates will come.
    pub fn is_active(self) -> bool {
        matches!(self, DownloadState::InProgress | DownloadState::Paused)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DownloadRecord {
    pub id: Uuid,
    /// The engine's id for the download; only meaningful this session.
    #[serde(skip)]
    pub engine_id: Option<u32>,
    pub url: String,
    pub filename: String,
    pub path: PathBuf,
    pub received: i64,
    /// -1 or 0 when unknown.
    pub total: i64,
    /// 0-100, or -1 when the size is unknown.
    #[serde(skip)]
    pub percent: i32,
    /// Bytes per second, while running.
    #[serde(skip)]
    pub speed: i64,
    pub state: DownloadState,
    pub started_at: f64,
    pub finished_at: Option<f64>,
}

impl Default for DownloadRecord {
    fn default() -> Self {
        DownloadRecord {
            id: Uuid::new_v4(),
            engine_id: None,
            url: String::new(),
            filename: String::new(),
            path: PathBuf::new(),
            received: 0,
            total: 0,
            percent: -1,
            speed: 0,
            state: DownloadState::InProgress,
            started_at: 0.0,
            finished_at: None,
        }
    }
}

impl DownloadRecord {
    /// Fraction done in `0..=1`, or `None` when the size is unknown.
    pub fn fraction(&self) -> Option<f32> {
        if self.percent >= 0 {
            Some(self.percent.min(100) as f32 / 100.0)
        } else if self.total > 0 {
            Some((self.received as f64 / self.total as f64).clamp(0.0, 1.0) as f32)
        } else {
            None
        }
    }
}

/// One progress report from the engine.
#[derive(Clone, Debug, PartialEq)]
pub struct DownloadProgress {
    pub engine_id: u32,
    pub url: String,
    pub filename: String,
    pub path: PathBuf,
    pub received: i64,
    pub total: i64,
    pub percent: i32,
    pub speed: i64,
    pub state: DownloadState,
}

/// What an update changed, for the chrome to react to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DownloadChange {
    Started,
    Progressed,
    /// The download just reached `state` (complete, canceled, failed).
    Finished(DownloadState),
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DownloadStore {
    /// Newest first.
    pub entries: Vec<DownloadRecord>,
}

impl DownloadStore {
    /// Load `downloads.json`. Downloads that were running when Mori last
    /// quit cannot resume and are marked interrupted.
    pub fn load(dir: &Path) -> DownloadStore {
        let mut store: DownloadStore = paths::read_json(&dir.join(DOWNLOADS_FILE)).unwrap_or_default();
        for entry in &mut store.entries {
            if entry.state.is_active() {
                entry.state = DownloadState::Interrupted;
                entry.speed = 0;
            }
        }
        store
    }

    pub fn save(&self, dir: &Path) -> anyhow::Result<()> {
        paths::write_json_atomic(&dir.join(DOWNLOADS_FILE), self)
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &DownloadRecord> {
        self.entries.iter()
    }

    pub fn active_count(&self) -> usize {
        self.entries.iter().filter(|e| e.state.is_active()).count()
    }

    /// Overall progress of running downloads in `0..=1`, if any report a size.
    pub fn active_fraction(&self) -> Option<f32> {
        let (done, total) = self
            .entries
            .iter()
            .filter(|e| e.state.is_active() && e.total > 0)
            .fold((0i64, 0i64), |(d, t), e| (d + e.received, t + e.total));
        (total > 0).then(|| (done as f64 / total as f64).clamp(0.0, 1.0) as f32)
    }

    pub fn find_engine(&self, engine_id: u32) -> Option<&DownloadRecord> {
        self.entries.iter().find(|e| e.engine_id == Some(engine_id))
    }

    /// Merge an engine update, returning what changed.
    pub fn apply(&mut self, update: DownloadProgress, now: f64) -> DownloadChange {
        let finished_at = (!update.state.is_active()).then_some(now);
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|e| e.engine_id == Some(update.engine_id))
        {
            let was_active = entry.state.is_active();
            entry.url = update.url;
            entry.filename = update.filename;
            entry.path = update.path;
            entry.received = update.received;
            entry.total = update.total;
            entry.percent = update.percent;
            entry.speed = if update.state == DownloadState::InProgress { update.speed } else { 0 };
            entry.state = update.state;
            if was_active && !update.state.is_active() {
                entry.finished_at = finished_at;
                return DownloadChange::Finished(update.state);
            }
            return DownloadChange::Progressed;
        }
        let state = update.state;
        self.entries.insert(
            0,
            DownloadRecord {
                id: Uuid::new_v4(),
                engine_id: Some(update.engine_id),
                url: update.url,
                filename: update.filename,
                path: update.path,
                received: update.received,
                total: update.total,
                percent: update.percent,
                speed: update.speed,
                state,
                started_at: now,
                finished_at,
            },
        );
        self.trim();
        if state.is_active() {
            DownloadChange::Started
        } else {
            DownloadChange::Finished(state)
        }
    }

    /// Record a file Mori saved itself (a page printed to PDF) as a
    /// finished download.
    pub fn add_file(&mut self, url: &str, path: &Path, size: i64, now: f64) {
        let filename = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.entries.insert(
            0,
            DownloadRecord {
                url: url.to_string(),
                filename,
                path: path.to_path_buf(),
                received: size,
                total: size,
                percent: 100,
                state: DownloadState::Complete,
                started_at: now,
                finished_at: Some(now),
                ..Default::default()
            },
        );
        self.trim();
    }

    pub fn remove(&mut self, id: Uuid) {
        self.entries.retain(|e| e.id != id);
    }

    /// Drop every finished entry, keeping running and paused downloads.
    pub fn clear_finished(&mut self) {
        self.entries.retain(|e| e.state.is_active());
    }

    /// Drop finished entries started since `since` (UNIX seconds), or all
    /// with `None`. Running and paused downloads stay.
    pub fn clear_finished_since(&mut self, since: Option<f64>) {
        self.entries
            .retain(|e| e.state.is_active() || since.is_some_and(|t| e.started_at < t));
    }

    /// Keep at most [`MAX_ENTRIES`], dropping the oldest finished ones.
    fn trim(&mut self) {
        while self.entries.len() > MAX_ENTRIES {
            match self.entries.iter().rposition(|e| !e.state.is_active()) {
                Some(i) => {
                    self.entries.remove(i);
                }
                None => break,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::test_support::TempDir;

    fn progress(id: u32, state: DownloadState, received: i64) -> DownloadProgress {
        DownloadProgress {
            engine_id: id,
            url: format!("https://example.com/{id}.bin"),
            filename: format!("{id}.bin"),
            path: PathBuf::from(format!("/tmp/{id}.bin")),
            received,
            total: 100,
            percent: received as i32,
            speed: 10,
            state,
        }
    }

    #[test]
    fn merges_updates_and_reports_changes() {
        let mut store = DownloadStore::default();
        assert_eq!(store.apply(progress(1, DownloadState::InProgress, 0), 1.0), DownloadChange::Started);
        assert_eq!(store.apply(progress(1, DownloadState::InProgress, 50), 2.0), DownloadChange::Progressed);
        assert_eq!(store.entries.len(), 1);
        assert_eq!(store.entries[0].received, 50);
        assert_eq!(store.active_count(), 1);
        assert_eq!(store.active_fraction(), Some(0.5));
        assert_eq!(
            store.apply(progress(1, DownloadState::Complete, 100), 3.0),
            DownloadChange::Finished(DownloadState::Complete)
        );
        assert_eq!(store.entries[0].finished_at, Some(3.0));
        assert_eq!(store.entries[0].speed, 0);
        // A repeated terminal update is not a second finish.
        assert_eq!(store.apply(progress(1, DownloadState::Complete, 100), 4.0), DownloadChange::Progressed);
        assert_eq!(store.active_count(), 0);
        assert_eq!(store.active_fraction(), None);
    }

    #[test]
    fn newest_first_and_trimmed() {
        let mut store = DownloadStore::default();
        for id in 0..(MAX_ENTRIES as u32 + 5) {
            store.apply(progress(id, DownloadState::Complete, 100), id as f64);
        }
        assert_eq!(store.entries.len(), MAX_ENTRIES);
        assert_eq!(store.entries[0].engine_id, Some(MAX_ENTRIES as u32 + 4));
    }

    #[test]
    fn trimming_keeps_running_downloads() {
        let mut store = DownloadStore::default();
        store.apply(progress(0, DownloadState::InProgress, 1), 0.0);
        for id in 1..=(MAX_ENTRIES as u32 + 3) {
            store.apply(progress(id, DownloadState::Complete, 100), id as f64);
        }
        assert_eq!(store.entries.len(), MAX_ENTRIES);
        assert!(store.find_engine(0).is_some());
    }

    #[test]
    fn clears_finished_in_a_time_range() {
        let mut store = DownloadStore::default();
        store.apply(progress(1, DownloadState::Complete, 100), 10.0);
        store.apply(progress(2, DownloadState::Complete, 100), 100.0);
        store.apply(progress(3, DownloadState::InProgress, 5), 120.0);
        store.clear_finished_since(Some(50.0));
        let left: Vec<_> = store.entries.iter().filter_map(|e| e.engine_id).collect();
        assert_eq!(left, [3, 1], "old and running entries stay");
        store.clear_finished_since(None);
        assert_eq!(store.entries.len(), 1);
    }

    #[test]
    fn clear_finished_keeps_active() {
        let mut store = DownloadStore::default();
        store.apply(progress(1, DownloadState::Complete, 100), 0.0);
        store.apply(progress(2, DownloadState::Paused, 10), 0.0);
        store.apply(progress(3, DownloadState::Canceled, 10), 0.0);
        store.clear_finished();
        assert_eq!(store.entries.len(), 1);
        assert_eq!(store.entries[0].engine_id, Some(2));
    }

    #[test]
    fn reload_marks_running_downloads_interrupted() {
        let tmp = TempDir::new();
        let mut store = DownloadStore::default();
        store.apply(progress(1, DownloadState::InProgress, 10), 0.0);
        store.apply(progress(2, DownloadState::Complete, 100), 0.0);
        store.save(tmp.path()).unwrap();

        let loaded = DownloadStore::load(tmp.path());
        assert_eq!(loaded.entries.len(), 2);
        assert_eq!(loaded.entries[1].state, DownloadState::Interrupted);
        assert_eq!(loaded.entries[0].state, DownloadState::Complete);
        // Engine ids do not survive a restart.
        assert!(loaded.entries.iter().all(|e| e.engine_id.is_none()));
    }

    #[test]
    fn add_file_records_a_finished_entry() {
        let mut store = DownloadStore::default();
        store.apply(progress(1, DownloadState::InProgress, 1), 0.0);
        store.add_file("https://example.com/", Path::new("/tmp/Page.pdf"), 42, 5.0);
        let entry = &store.entries[0];
        assert_eq!(entry.filename, "Page.pdf");
        assert_eq!(entry.state, DownloadState::Complete);
        assert_eq!(entry.engine_id, None);
        assert_eq!(entry.fraction(), Some(1.0));
        // It never collides with engine updates.
        assert_eq!(store.apply(progress(1, DownloadState::InProgress, 2), 6.0), DownloadChange::Progressed);
    }

    #[test]
    fn fraction_falls_back_to_bytes() {
        let mut record = DownloadRecord { received: 25, total: 100, ..Default::default() };
        assert_eq!(record.fraction(), Some(0.25));
        record.total = -1;
        assert_eq!(record.fraction(), None);
        record.percent = 80;
        assert_eq!(record.fraction(), Some(0.8));
    }
}
