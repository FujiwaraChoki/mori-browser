//! Importing from other browsers: Chrome-family (Chrome, Chromium, Brave,
//! Vivaldi, Edge), Firefox-family (Firefox, Zen) and Arc.
//!
//! [`detect`] finds installed browser profiles, [`read`] turns one into a
//! browser-neutral [`Imported`] value, and [`apply`] merges that into Mori's
//! stores. Other browsers' files are never written: databases are copied to
//! a temporary directory and opened there, everything else is only read.
//!
//! Mapping (Arc's onboarding does the same):
//! - bookmarks → the bookmarks list (History & Bookmarks),
//! - history → history,
//! - open tabs and saved sessions → a space per window or workspace, with
//!   pinned tabs pinned and tab groups / folders as folders,
//! - Zen essentials and Arc favorites ("top apps") → pinned tiles of the
//!   first imported space.

mod arc;
mod chromium;
mod firefox;

pub use arc::SIDEBAR_FILE as ARC_SIDEBAR_FILE;

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use crate::{
    bookmarks::{Bookmark, BookmarkStore},
    history::{self, HistoryEntry, HistoryStore, MAX_HISTORY_ENTRIES},
    workspace::{Slot, SpaceId, TabRecord, Workspace},
};

/// Most history entries read from another browser (newest first); Mori keeps
/// no more than this anyway.
pub const HISTORY_LIMIT: usize = MAX_HISTORY_ENTRIES;

/// A browser Mori can import from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BrowserKind {
    Chrome,
    Chromium,
    Brave,
    Vivaldi,
    Edge,
    Firefox,
    Zen,
    Arc,
}

impl BrowserKind {
    pub fn label(self) -> &'static str {
        match self {
            BrowserKind::Chrome => "Google Chrome",
            BrowserKind::Chromium => "Chromium",
            BrowserKind::Brave => "Brave",
            BrowserKind::Vivaldi => "Vivaldi",
            BrowserKind::Edge => "Microsoft Edge",
            BrowserKind::Firefox => "Firefox",
            BrowserKind::Zen => "Zen",
            BrowserKind::Arc => "Arc",
        }
    }

    /// Short name for space names ("Chrome", "Zen").
    pub fn short_label(self) -> &'static str {
        match self {
            BrowserKind::Chrome => "Chrome",
            BrowserKind::Edge => "Edge",
            other => other.label(),
        }
    }

    fn family(self) -> Family {
        match self {
            BrowserKind::Chrome
            | BrowserKind::Chromium
            | BrowserKind::Brave
            | BrowserKind::Vivaldi
            | BrowserKind::Edge => Family::Chromium,
            BrowserKind::Firefox | BrowserKind::Zen => Family::Firefox,
            BrowserKind::Arc => Family::Arc,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Family {
    Chromium,
    Firefox,
    Arc,
}

/// What a source can provide.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Contents {
    pub bookmarks: bool,
    pub history: bool,
    /// Open tabs or a saved session (Zen workspaces and pinned tabs, Arc
    /// spaces).
    pub tabs: bool,
}

impl Contents {
    pub fn any(&self) -> bool {
        self.bookmarks || self.history || self.tabs
    }

    /// "Bookmarks, history and tabs".
    pub fn describe(&self) -> String {
        let parts: Vec<&str> = [
            (self.bookmarks, "bookmarks"),
            (self.history, "history"),
            (self.tabs, "tabs"),
        ]
        .iter()
        .filter(|(on, _)| *on)
        .map(|(_, name)| *name)
        .collect();
        let text = match parts.as_slice() {
            [] => String::new(),
            [one] => one.to_string(),
            [init @ .., last] => format!("{} and {last}", init.join(", ")),
        };
        let mut chars = text.chars();
        chars
            .next()
            .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
            .unwrap_or_default()
    }
}

/// One importable browser profile (or, for Arc, one sidebar file).
#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    pub kind: BrowserKind,
    /// The profile's name as the browser shows it ("Default", "Work").
    pub profile: String,
    /// Profile directory, or the `StorableSidebar.json` file for Arc.
    pub path: PathBuf,
    pub contents: Contents,
}

impl Source {
    /// "Google Chrome" or "Google Chrome — Work" when the browser has
    /// several profiles.
    pub fn title(&self, show_profile: bool) -> String {
        if show_profile {
            format!("{} — {}", self.kind.label(), self.profile)
        } else {
            self.kind.label().to_string()
        }
    }
}

/// What to bring over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    pub bookmarks: bool,
    pub history: bool,
    pub tabs: bool,
}

impl Options {
    pub const ALL: Options = Options {
        bookmarks: true,
        history: true,
        tabs: true,
    };

    /// The options a source can actually honour.
    pub fn limited_to(self, contents: Contents) -> Options {
        Options {
            bookmarks: self.bookmarks && contents.bookmarks,
            history: self.history && contents.history,
            tabs: self.tabs && contents.tabs,
        }
    }

    pub fn any(&self) -> bool {
        self.bookmarks || self.history || self.tabs
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImportedBookmark {
    pub title: String,
    pub url: String,
    /// UNIX seconds; 0 when unknown.
    pub added: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImportedVisit {
    pub url: String,
    pub title: String,
    /// UNIX seconds.
    pub last_visited: f64,
    pub visit_count: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImportedTab {
    pub url: String,
    pub title: String,
}

impl ImportedTab {
    pub fn new(url: impl Into<String>, title: impl Into<String>) -> Self {
        ImportedTab {
            url: url.into(),
            title: title.into(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImportedFolder {
    pub name: String,
    pub tabs: Vec<ImportedTab>,
}

/// A window, Zen workspace or Arc space.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImportedSpace {
    pub name: String,
    pub pinned: Vec<ImportedTab>,
    pub folders: Vec<ImportedFolder>,
    /// Ordinary open tabs, top first.
    pub tabs: Vec<ImportedTab>,
}

impl ImportedSpace {
    fn is_empty(&self) -> bool {
        self.pinned.is_empty() && self.tabs.is_empty() && self.folders.iter().all(|f| f.tabs.is_empty())
    }
}

/// Everything read from one source.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Imported {
    pub bookmarks: Vec<ImportedBookmark>,
    pub history: Vec<ImportedVisit>,
    /// Zen essentials and Arc favorites.
    pub favorites: Vec<ImportedTab>,
    pub spaces: Vec<ImportedSpace>,
    /// Parts that could not be read, for the summary.
    pub problems: Vec<String>,
}

/// What [`apply`] added.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub bookmarks: usize,
    pub history: usize,
    pub tabs: usize,
    pub spaces: usize,
}

impl Summary {
    pub fn is_empty(&self) -> bool {
        *self == Summary::default()
    }

    /// "120 bookmarks, 2,000 history entries and 34 tabs in 2 spaces".
    pub fn describe(&self) -> String {
        let plural = |n: usize, one: &str, many: &str| format!("{} {}", group_digits(n), if n == 1 { one } else { many });
        let mut parts = Vec::new();
        if self.bookmarks > 0 {
            parts.push(plural(self.bookmarks, "bookmark", "bookmarks"));
        }
        if self.history > 0 {
            parts.push(plural(self.history, "history entry", "history entries"));
        }
        if self.tabs > 0 {
            let mut tabs = plural(self.tabs, "tab", "tabs");
            if self.spaces > 0 {
                tabs.push_str(&format!(" in {}", plural(self.spaces, "new space", "new spaces")));
            }
            parts.push(tabs);
        }
        match parts.as_slice() {
            [] => "Nothing new to import".to_string(),
            [one] => one.clone(),
            [init @ .., last] => format!("{} and {last}", init.join(", ")),
        }
    }
}

fn group_digits(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A step of [`read`], for progress reporting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Bookmarks,
    History,
    Tabs,
}

impl Step {
    pub fn label(self) -> &'static str {
        match self {
            Step::Bookmarks => "Reading bookmarks…",
            Step::History => "Reading history…",
            Step::Tabs => "Reading tabs…",
        }
    }
}

/// Find importable profiles under `home`, in a stable order (Chrome-family,
/// then Firefox-family, then Arc).
pub fn detect(home: &Path) -> Vec<Source> {
    let mut out = chromium::detect(home);
    out.extend(firefox::detect(home));
    out.extend(arc::detect(home));
    out.retain(|s| s.contents.any());
    out
}

/// The source to offer first: the one with the most to import (bookmarks,
/// history and tabs), the earliest found on a tie.
pub fn suggested(sources: &[Source]) -> usize {
    let score = |c: &Contents| c.bookmarks as u8 + c.history as u8 + c.tabs as u8;
    let mut best = 0;
    for (i, s) in sources.iter().enumerate() {
        if score(&s.contents) > score(&sources[best].contents) {
            best = i;
        }
    }
    best
}

/// Read `source`. `progress` is called before each step with the step and
/// `(index, total)`. A part that fails is recorded in
/// [`Imported::problems`] and the rest is still read.
pub fn read(source: &Source, options: Options, progress: &mut dyn FnMut(Step, usize, usize)) -> Imported {
    let options = options.limited_to(source.contents);
    let steps: Vec<Step> = [
        (options.bookmarks, Step::Bookmarks),
        (options.history, Step::History),
        (options.tabs, Step::Tabs),
    ]
    .into_iter()
    .filter(|(on, _)| *on)
    .map(|(_, s)| s)
    .collect();
    let mut imported = Imported::default();
    for (i, step) in steps.iter().enumerate() {
        progress(*step, i, steps.len());
        let result = match source.kind.family() {
            Family::Chromium => chromium::read(source, *step, &mut imported),
            Family::Firefox => firefox::read(source, *step, &mut imported),
            Family::Arc => arc::read(source, *step, &mut imported),
        };
        if let Err(e) = result {
            log::warn!("import {:?} {step:?} from {}: {e:#}", source.kind, source.path.display());
            let what = match step {
                Step::Bookmarks => "bookmarks",
                Step::History => "history",
                Step::Tabs => "tabs",
            };
            imported.problems.push(format!("Couldn't read {what}"));
        }
    }
    imported.spaces.retain(|s| !s.is_empty());
    imported
}

/// Merge `imported` into Mori's stores. Nothing is duplicated: bookmarks and
/// history merge by URL, a space whose name already exists receives only
/// the tabs it doesn't have yet, and favorites already pinned are skipped.
/// Imported spaces are named "<browser> · <name>" (or just the browser).
pub fn apply(
    kind: BrowserKind,
    imported: &Imported,
    ws: &mut Workspace,
    bookmarks: &mut BookmarkStore,
    history: &mut HistoryStore,
) -> Summary {
    let mut summary = Summary {
        bookmarks: merge_bookmarks(bookmarks, &imported.bookmarks),
        history: merge_history(history, &imported.history),
        ..Summary::default()
    };
    let single = imported.spaces.len() == 1;
    let mut first_space: Option<SpaceId> = None;
    for space in &imported.spaces {
        let name = space_name(kind, &space.name, single);
        let (id, created) = match ws.spaces.iter().find(|s| s.name == name) {
            Some(s) => (s.id, false),
            None => (ws.create_space(name), true),
        };
        let added = fill_space(ws, id, space);
        if created {
            if added == 0 {
                // Everything in it was already open: no empty space.
                ws.delete_space(id);
                continue;
            }
            summary.spaces += 1;
        }
        summary.tabs += added;
        first_space.get_or_insert(id);
    }
    if !imported.favorites.is_empty() {
        let target = first_space.unwrap_or(ws.active_space);
        let mut pinned: HashSet<String> = ws
            .spaces
            .iter()
            .flat_map(|s| s.pinned.iter())
            .filter_map(|t| ws.tab(*t).map(|r| pin_key(&r.url)))
            .collect();
        let fresh = imported
            .favorites
            .iter()
            .filter(|t| is_importable_url(&t.url) && pinned.insert(pin_key(&t.url)));
        // Favorites lead the grid, in their own order.
        for (i, tab) in fresh.enumerate() {
            ws.add_tab(record_for(tab), Some(target), Slot::Pinned(i));
            summary.tabs += 1;
        }
    }
    summary
}

fn space_name(kind: BrowserKind, name: &str, single: bool) -> String {
    let name = name.trim();
    if name.is_empty() || (single && is_generic_space_name(name)) {
        kind.short_label().to_string()
    } else {
        format!("{} · {name}", kind.short_label())
    }
}

/// Names browsers give spaces and windows by default, which add nothing
/// after the browser's name.
fn is_generic_space_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower == "space" || lower == "default" || lower.starts_with("window")
}

/// Add `space`'s tabs to Mori space `id`, skipping URLs it already has.
/// Returns how many tabs were added.
fn fill_space(ws: &mut Workspace, id: SpaceId, space: &ImportedSpace) -> usize {
    let urls = |tabs: Vec<crate::workspace::TabId>| -> Vec<String> {
        tabs.into_iter().filter_map(|t| ws.tab(t).map(|r| r.url.clone())).collect()
    };
    let mut have: HashSet<String> = urls(ws.space(id).map(|s| s.ordered()).unwrap_or_default())
        .iter()
        .map(|u| page_key(u))
        .collect();
    let mut pins: HashSet<String> = urls(ws.space(id).map(|s| s.pinned.clone()).unwrap_or_default())
        .iter()
        .map(|u| pin_key(u))
        .collect();
    let mut added = 0;
    let take = |tab: &ImportedTab, have: &mut HashSet<String>| {
        (is_importable_url(&tab.url) && have.insert(page_key(&tab.url))).then(|| record_for(tab))
    };
    let pinned_start = ws.space(id).map(|s| s.pinned.len()).unwrap_or(0);
    let mut pinned = 0;
    for tab in &space.pinned {
        // A pinned tab wanders within its site: one per site.
        if !pins.insert(pin_key(&tab.url)) {
            continue;
        }
        if let Some(record) = take(tab, &mut have) {
            ws.add_tab(record, Some(id), Slot::Pinned(pinned_start + pinned));
            pinned += 1;
        }
    }
    added += pinned;
    for folder in &space.folders {
        let records: Vec<TabRecord> = folder.tabs.iter().filter_map(|t| take(t, &mut have)).collect();
        if records.is_empty() {
            continue;
        }
        let ids: Vec<_> = records
            .into_iter()
            .map(|r| ws.add_tab(r, Some(id), Slot::Loose(usize::MAX)))
            .collect();
        added += ids.len();
        let name = if folder.name.trim().is_empty() { "Folder" } else { folder.name.trim() };
        if let Some(fid) = ws.create_folder(id, name, &ids) {
            // Imported collections start closed, so the sidebar stays tidy.
            ws.update_folder(fid, |f| f.is_expanded = false);
        }
    }
    let loose_start = ws.space(id).map(|s| s.loose.len()).unwrap_or(0);
    let mut loose = 0;
    for tab in &space.tabs {
        if let Some(record) = take(tab, &mut have) {
            ws.add_tab(record, Some(id), Slot::Loose(loose_start + loose));
            loose += 1;
        }
    }
    added + loose
}

/// What makes two tabs the same page: the URL without its fragment or a
/// trailing slash.
fn page_key(url: &str) -> String {
    let url = url.split('#').next().unwrap_or(url);
    url.trim_end_matches('/').to_ascii_lowercase()
}

/// What makes two pinned tabs the same pin: their site (host without
/// `www.`), since pinned tabs move around within it.
fn pin_key(url: &str) -> String {
    crate::util::host_of(url)
        .map(|h| crate::util::strip_www(&h).to_string())
        .unwrap_or_else(|| page_key(url))
}

/// An unloaded tab for `tab`; it shows its imported title until it loads.
/// It counts as used at import time, so the archive rules give it the
/// usual grace period instead of archiving it right away.
fn record_for(tab: &ImportedTab) -> TabRecord {
    let mut record = TabRecord::new(tab.url.clone());
    record.title = tab.title.clone();
    record
}

/// Add bookmarks whose URL isn't bookmarked yet; the list stays newest
/// first. Returns how many were added.
pub fn merge_bookmarks(store: &mut BookmarkStore, imported: &[ImportedBookmark]) -> usize {
    let mut have: HashSet<String> = store.bookmarks.iter().map(|b| b.url.clone()).collect();
    let now = crate::paths::now();
    let mut added = 0;
    for b in imported {
        if !is_importable_url(&b.url) || !have.insert(b.url.clone()) {
            continue;
        }
        store.bookmarks.push(Bookmark {
            id: uuid::Uuid::new_v4(),
            url: b.url.clone(),
            title: if b.title.trim().is_empty() { b.url.clone() } else { b.title.clone() },
            created_at: if b.added > 0.0 { b.added } else { now },
        });
        added += 1;
    }
    if added > 0 {
        store.bookmarks.sort_by(|a, b| b.created_at.total_cmp(&a.created_at));
    }
    added
}

/// Merge visits by URL (latest visit and the higher count win, an empty
/// title is filled in), keeping the newest [`MAX_HISTORY_ENTRIES`]. Returns
/// how many URLs are new.
pub fn merge_history(store: &mut HistoryStore, imported: &[ImportedVisit]) -> usize {
    use std::collections::HashMap;
    let mut index: HashMap<String, usize> =
        store.entries.iter().enumerate().map(|(i, e)| (e.url.clone(), i)).collect();
    let mut added: usize = 0;
    for v in imported {
        if !history::is_recordable(&v.url) || !is_importable_url(&v.url) {
            continue;
        }
        match index.get(&v.url) {
            Some(&i) => {
                let e = &mut store.entries[i];
                e.last_visited = e.last_visited.max(v.last_visited);
                e.visit_count = e.visit_count.max(v.visit_count);
                if e.title.trim().is_empty() {
                    e.title = v.title.clone();
                }
            }
            None => {
                index.insert(v.url.clone(), store.entries.len());
                store.entries.push(HistoryEntry {
                    id: uuid::Uuid::new_v4(),
                    url: v.url.clone(),
                    title: v.title.clone(),
                    last_visited: v.last_visited,
                    visit_count: v.visit_count.max(1),
                });
                added += 1;
            }
        }
    }
    store.entries.sort_by(|a, b| b.last_visited.total_cmp(&a.last_visited));
    if store.entries.len() > MAX_HISTORY_ENTRIES {
        let dropped = store.entries.split_off(MAX_HISTORY_ENTRIES);
        let gone: HashSet<&str> = dropped.iter().map(|e| e.url.as_str()).collect();
        // Only count imported URLs that survived the cut.
        let lost = imported.iter().filter(|v| gone.contains(v.url.as_str())).count();
        added = added.saturating_sub(lost);
    }
    added
}

/// Web pages and local files only: internal pages of the other browser
/// (`chrome://`, `about:`, `place:`) mean nothing in Mori.
pub fn is_importable_url(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("file://")
}

/// Copy a database (and its `-wal` / `-journal` side file, if any) into a
/// fresh temporary directory and open the copy, so the other browser's file
/// is never locked or changed (and a running browser's lock doesn't stop
/// the import). The copy is deleted when this drops.
pub(crate) struct DbCopy {
    conn: Option<rusqlite::Connection>,
    _dir: TempFiles,
}

impl DbCopy {
    pub(crate) fn open(path: &Path) -> anyhow::Result<DbCopy> {
        use anyhow::Context as _;
        let dir = TempFiles(std::env::temp_dir().join(format!("mori-import-{}", uuid::Uuid::new_v4())));
        // Private: the copies hold the user's browsing history.
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder.create(&dir.0).with_context(|| format!("creating {}", dir.0.display()))?;
        let name = path.file_name().map(|n| n.to_os_string()).unwrap_or_else(|| "db".into());
        let copy = dir.0.join(&name);
        std::fs::copy(path, &copy).with_context(|| format!("copying {}", path.display()))?;
        for suffix in ["-wal", "-journal"] {
            let side = with_suffix(path, suffix);
            if side.exists() {
                // A journal that vanished meanwhile just means less to replay.
                let _ = std::fs::copy(&side, with_suffix(&copy, suffix));
            }
        }
        let conn = rusqlite::Connection::open(&copy).with_context(|| format!("opening copy of {}", path.display()))?;
        Ok(DbCopy { conn: Some(conn), _dir: dir })
    }

    pub(crate) fn conn(&self) -> &rusqlite::Connection {
        self.conn.as_ref().expect("open until dropped")
    }
}

impl Drop for DbCopy {
    fn drop(&mut self) {
        // Close the database before its files are removed.
        self.conn.take();
    }
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(suffix);
    PathBuf::from(s)
}

/// A flat temporary directory, removed with its files on drop.
struct TempFiles(PathBuf);

impl Drop for TempFiles {
    fn drop(&mut self) {
        if let Ok(entries) = std::fs::read_dir(&self.0) {
            for entry in entries.flatten() {
                let _ = std::fs::remove_file(entry.path());
            }
        }
        let _ = std::fs::remove_dir(&self.0);
    }
}

/// Read a JSON file into a value.
fn read_json_file(path: &Path) -> anyhow::Result<serde_json::Value> {
    use anyhow::Context as _;
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", path.display()))
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::path::{Path, PathBuf};

    /// A scratch tree under the temp dir; files and directories created
    /// through it are removed (deepest first) on drop.
    pub struct Tree {
        pub root: PathBuf,
        created: Vec<PathBuf>,
    }

    impl Tree {
        pub fn new() -> Tree {
            let root = std::env::temp_dir().join(format!("mori-import-test-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&root).unwrap();
            Tree { root, created: Vec::new() }
        }

        /// Write `bytes` at `rel`, creating parent directories.
        pub fn write(&mut self, rel: &str, bytes: impl AsRef<[u8]>) -> PathBuf {
            let path = self.root.join(rel);
            let mut missing = Vec::new();
            let mut dir = path.parent();
            while let Some(d) = dir {
                if d.exists() {
                    break;
                }
                missing.push(d.to_path_buf());
                dir = d.parent();
            }
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            self.created.extend(missing.into_iter().rev());
            std::fs::write(&path, bytes).unwrap();
            self.created.push(path.clone());
            path
        }

        /// Register a file created by other means (e.g. SQLite) for cleanup.
        pub fn track(&mut self, path: &Path) {
            self.created.push(path.to_path_buf());
        }

        pub fn path(&self, rel: &str) -> PathBuf {
            self.root.join(rel)
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            for p in self.created.iter().rev() {
                if p.is_dir() {
                    let _ = std::fs::remove_dir(p);
                } else {
                    let _ = std::fs::remove_file(p);
                }
            }
            // SQLite side files next to tracked databases.
            if let Ok(entries) = std::fs::read_dir(&self.root) {
                for e in entries.flatten() {
                    if e.path().is_file() {
                        let _ = std::fs::remove_file(e.path());
                    }
                }
            }
            let _ = std::fs::remove_dir(&self.root);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn visit(url: &str, title: &str, at: f64, count: u32) -> ImportedVisit {
        ImportedVisit {
            url: url.into(),
            title: title.into(),
            last_visited: at,
            visit_count: count,
        }
    }

    #[test]
    fn bookmarks_merge_by_url_newest_first() {
        let mut store = BookmarkStore::default();
        store.toggle("https://a.test/", "A");
        let a_time = store.bookmarks[0].created_at;
        let imported = vec![
            ImportedBookmark { title: "A again".into(), url: "https://a.test/".into(), added: 5.0 },
            ImportedBookmark { title: "".into(), url: "https://b.test/".into(), added: 10.0 },
            ImportedBookmark { title: "Settings".into(), url: "chrome://settings".into(), added: 11.0 },
            ImportedBookmark { title: "B dup".into(), url: "https://b.test/".into(), added: 12.0 },
        ];
        assert_eq!(merge_bookmarks(&mut store, &imported), 1);
        assert_eq!(store.bookmarks.len(), 2);
        // Existing (just now) stays above the old imported one.
        assert_eq!(store.bookmarks[0].created_at, a_time);
        assert_eq!(store.bookmarks[1].url, "https://b.test/");
        assert_eq!(store.bookmarks[1].title, "https://b.test/", "empty title falls back to the URL");
        assert_eq!(merge_bookmarks(&mut store, &imported), 0, "importing twice adds nothing");
    }

    #[test]
    fn history_merges_and_stays_bounded() {
        let mut store = HistoryStore::default();
        store.record_at("https://a.test/", "", 100.0);
        let imported = vec![
            visit("https://a.test/", "Alpha", 50.0, 7),
            visit("https://b.test/", "Beta", 200.0, 2),
            visit("about:blank", "", 300.0, 1),
        ];
        assert_eq!(merge_history(&mut store, &imported), 1);
        assert_eq!(store.entries[0].url, "https://b.test/");
        let a = &store.entries[1];
        assert_eq!((a.last_visited, a.visit_count, a.title.as_str()), (100.0, 7, "Alpha"));

        let many: Vec<ImportedVisit> = (0..MAX_HISTORY_ENTRIES + 10)
            .map(|i| visit(&format!("https://x.test/{i}"), "", i as f64, 1))
            .collect();
        let added = merge_history(&mut store, &many);
        assert_eq!(store.entries.len(), MAX_HISTORY_ENTRIES);
        // b.test (200) survives; a.test (100) and the 100 oldest imports fall off.
        assert!(store.entries.iter().any(|e| e.url == "https://b.test/"));
        assert!(added < MAX_HISTORY_ENTRIES);
        assert_eq!(added, store.entries.iter().filter(|e| e.url.starts_with("https://x.test/")).count());
    }

    fn sample() -> Imported {
        Imported {
            favorites: vec![ImportedTab::new("https://mail.test/", "Mail")],
            spaces: vec![ImportedSpace {
                name: "Work".into(),
                pinned: vec![ImportedTab::new("https://docs.test/", "Docs")],
                folders: vec![ImportedFolder {
                    name: "Providers".into(),
                    tabs: vec![
                        ImportedTab::new("https://one.test/", "One"),
                        ImportedTab::new("https://two.test/", "Two"),
                    ],
                }],
                tabs: vec![
                    ImportedTab::new("https://news.test/", "News"),
                    ImportedTab::new("https://news.test/", "News twice"),
                ],
            }],
            ..Imported::default()
        }
    }

    #[test]
    fn spaces_folders_and_favorites_apply_once() {
        let mut ws = Workspace::new();
        let (mut b, mut h) = (BookmarkStore::default(), HistoryStore::default());
        let before = ws.spaces.len();
        let summary = apply(BrowserKind::Zen, &sample(), &mut ws, &mut b, &mut h);
        assert_eq!(summary.spaces, 1);
        assert_eq!(summary.tabs, 5, "the duplicate News tab is dropped");
        assert_eq!(ws.spaces.len(), before + 1);
        let space = ws.spaces.last().unwrap();
        assert_eq!(space.name, "Zen · Work");
        let url = |id| ws.tab(id).unwrap().url.clone();
        // The favorite leads the pinned grid, then the space's own pins.
        assert_eq!(space.pinned.iter().map(|t| url(*t)).collect::<Vec<_>>(), [
            "https://mail.test/",
            "https://docs.test/"
        ]);
        assert_eq!(space.folders.len(), 1);
        assert_eq!(space.folders[0].name, "Providers");
        assert!(!space.folders[0].is_expanded);
        assert_eq!(space.folders[0].tab_ids.len(), 2);
        assert_eq!(space.loose.iter().map(|t| url(*t)).collect::<Vec<_>>(), ["https://news.test/"]);
        assert_eq!(ws.tab(space.loose[0]).unwrap().title, "News");
        // Fresh, not idle: the archive must not take imported tabs at once.
        assert!(crate::paths::now() - ws.tab(space.loose[0]).unwrap().last_accessed < 60.0);

        // Again: same space, nothing new, no extra space, even after the
        // pinned tabs moved within their sites and a tab gained a fragment.
        let pins = ws.spaces.last().unwrap().pinned.clone();
        ws.tab_mut(pins[0]).unwrap().url = "https://mail.test/inbox/7".into();
        ws.tab_mut(pins[1]).unwrap().url = "https://www.docs.test/d/1".into();
        let news = ws.spaces.last().unwrap().loose[0];
        ws.tab_mut(news).unwrap().url = "https://news.test#top".into();
        let again = apply(BrowserKind::Zen, &sample(), &mut ws, &mut b, &mut h);
        assert!(again.is_empty(), "{again:?}");
        assert_eq!(ws.spaces.len(), before + 1);
    }

    #[test]
    fn a_single_default_window_is_named_after_the_browser() {
        assert_eq!(space_name(BrowserKind::Chrome, "Window 1", true), "Chrome");
        assert_eq!(space_name(BrowserKind::Chrome, "Window 2", false), "Chrome · Window 2");
        assert_eq!(space_name(BrowserKind::Zen, "Space", true), "Zen");
        assert_eq!(space_name(BrowserKind::Arc, "Personal", true), "Arc · Personal");
        assert_eq!(space_name(BrowserKind::Firefox, " ", false), "Firefox");
    }

    #[test]
    fn a_space_with_only_known_tabs_is_not_created() {
        let mut ws = Workspace::new();
        let (mut b, mut h) = (BookmarkStore::default(), HistoryStore::default());
        let mut imported = sample();
        imported.favorites.clear();
        apply(BrowserKind::Chrome, &imported, &mut ws, &mut b, &mut h);
        let spaces = ws.spaces.len();
        // Same tabs under another window name: the new space would be empty
        // only if its tabs were known to it; a different name is a new space.
        imported.spaces[0].name = "Other".into();
        let s = apply(BrowserKind::Chrome, &imported, &mut ws, &mut b, &mut h);
        assert_eq!(s.spaces, 1);
        assert_eq!(ws.spaces.len(), spaces + 1);
        // An empty imported space never creates one.
        imported.spaces[0] = ImportedSpace { name: "Empty".into(), ..Default::default() };
        let s = apply(BrowserKind::Chrome, &imported, &mut ws, &mut b, &mut h);
        assert_eq!(s.spaces, 0);
        assert_eq!(ws.spaces.len(), spaces + 1);
    }

    #[test]
    fn summaries_read_naturally() {
        assert_eq!(Summary::default().describe(), "Nothing new to import");
        let s = Summary { bookmarks: 1, history: 2000, tabs: 34, spaces: 2 };
        assert_eq!(s.describe(), "1 bookmark, 2,000 history entries and 34 tabs in 2 new spaces");
        assert_eq!(Summary { tabs: 3, ..Default::default() }.describe(), "3 tabs");
        let c = Contents { bookmarks: true, history: false, tabs: true };
        assert_eq!(c.describe(), "Bookmarks and tabs");
        assert_eq!(Contents { history: true, ..Default::default() }.describe(), "History");
    }

    #[test]
    fn options_follow_what_a_source_has() {
        let c = Contents { bookmarks: true, history: false, tabs: true };
        let o = Options::ALL.limited_to(c);
        assert_eq!(o, Options { bookmarks: true, history: false, tabs: true });
        assert!(!Options { bookmarks: false, history: false, tabs: false }.any());
    }

    #[test]
    fn the_richest_source_is_suggested() {
        let source = |kind, bookmarks, history, tabs| Source {
            kind,
            profile: "Default".into(),
            path: PathBuf::new(),
            contents: Contents { bookmarks, history, tabs },
        };
        let sources = vec![
            source(BrowserKind::Chrome, false, true, false),
            source(BrowserKind::Chromium, false, true, true),
            source(BrowserKind::Zen, true, true, true),
            source(BrowserKind::Firefox, true, true, true),
        ];
        assert_eq!(suggested(&sources), 2);
        assert_eq!(suggested(&sources[..2]), 1);
        assert_eq!(suggested(&[]), 0);
    }

    #[test]
    fn importable_urls() {
        assert!(is_importable_url("https://a.test"));
        assert!(is_importable_url("FILE:///home/x.html"));
        assert!(!is_importable_url("chrome://settings"));
        assert!(!is_importable_url("place:sort=8"));
        assert!(!is_importable_url("javascript:alert(1)"));
    }
}
