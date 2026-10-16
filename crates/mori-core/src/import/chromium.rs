//! Chrome-family profiles: `Bookmarks` (JSON), `History` (SQLite) and the
//! last session (`Sessions/Session_*`, Chromium's SNSS command log).

use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
};

use anyhow::{Context as _, bail};
use serde_json::Value;

use super::{
    BrowserKind, Contents, DbCopy, HISTORY_LIMIT, Imported, ImportedBookmark, ImportedSpace, ImportedTab,
    ImportedVisit, Source, Step, is_importable_url, read_json_file,
};

/// Seconds between 1601-01-01 (Chromium's epoch) and 1970-01-01.
const WEBKIT_EPOCH_OFFSET: f64 = 11_644_473_600.0;

/// User-data directories per browser, relative to the home directory.
fn roots() -> &'static [(BrowserKind, &'static str)] {
    if cfg!(target_os = "macos") {
        &[
            (BrowserKind::Chrome, "Library/Application Support/Google/Chrome"),
            (BrowserKind::Chromium, "Library/Application Support/Chromium"),
            (BrowserKind::Brave, "Library/Application Support/BraveSoftware/Brave-Browser"),
            (BrowserKind::Vivaldi, "Library/Application Support/Vivaldi"),
            (BrowserKind::Edge, "Library/Application Support/Microsoft Edge"),
        ]
    } else {
        &[
            (BrowserKind::Chrome, ".config/google-chrome"),
            (BrowserKind::Chrome, ".var/app/com.google.Chrome/config/google-chrome"),
            (BrowserKind::Chromium, ".config/chromium"),
            (BrowserKind::Chromium, ".var/app/org.chromium.Chromium/config/chromium"),
            (BrowserKind::Chromium, "snap/chromium/common/chromium"),
            (BrowserKind::Brave, ".config/BraveSoftware/Brave-Browser"),
            (BrowserKind::Brave, ".var/app/com.brave.Browser/config/BraveSoftware/Brave-Browser"),
            (BrowserKind::Vivaldi, ".config/vivaldi"),
            (BrowserKind::Edge, ".config/microsoft-edge"),
        ]
    }
}

pub(super) fn detect(home: &Path) -> Vec<Source> {
    let mut out = Vec::new();
    for (kind, rel) in roots() {
        let root = home.join(rel);
        for (dir, name) in profiles(&root) {
            let path = root.join(&dir);
            let contents = Contents {
                bookmarks: path.join("Bookmarks").is_file(),
                history: path.join("History").is_file(),
                tabs: latest_session(&path).is_some(),
            };
            out.push(Source {
                kind: *kind,
                profile: name,
                path,
                contents,
            });
        }
    }
    out
}

/// `(directory, display name)` of each profile in a user-data directory,
/// in the browser's own order (from `Local State`), else just `Default`.
fn profiles(root: &Path) -> Vec<(String, String)> {
    let state = read_json_file(&root.join("Local State")).ok();
    let cache = state.as_ref().and_then(|s| s.pointer("/profile/info_cache")).and_then(Value::as_object);
    let Some(cache) = cache else {
        return if root.join("Default").is_dir() {
            vec![("Default".to_string(), "Default".to_string())]
        } else {
            Vec::new()
        };
    };
    let mut dirs: Vec<String> = state
        .as_ref()
        .and_then(|s| s.pointer("/profile/profiles_order"))
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let mut rest: Vec<String> = cache.keys().filter(|k| !dirs.contains(k)).cloned().collect();
    rest.sort();
    dirs.extend(rest);
    dirs.into_iter()
        .filter(|d| cache.contains_key(d) && root.join(d).is_dir())
        .map(|d| {
            let name = cache
                .get(&d)
                .and_then(|p| p.get("name"))
                .and_then(Value::as_str)
                .filter(|n| !n.trim().is_empty())
                .unwrap_or(&d)
                .to_string();
            (d, name)
        })
        .collect()
}

pub(super) fn read(source: &Source, step: Step, out: &mut Imported) -> anyhow::Result<()> {
    match step {
        Step::Bookmarks => {
            let json = read_json_file(&source.path.join("Bookmarks"))?;
            out.bookmarks = parse_bookmarks(&json);
        }
        Step::History => {
            let db = DbCopy::open(&source.path.join("History"))?;
            out.history = read_history(db.conn())?;
        }
        Step::Tabs => {
            let path = latest_session(&source.path).context("no session file")?;
            let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
            out.spaces = parse_session(&bytes)?
                .into_iter()
                .enumerate()
                .map(|(i, w)| ImportedSpace {
                    name: format!("Window {}", i + 1),
                    pinned: w.pinned,
                    folders: Vec::new(),
                    tabs: w.tabs,
                })
                .collect();
        }
    }
    Ok(())
}

fn webkit_to_unix(micros: f64) -> f64 {
    if micros <= 0.0 { 0.0 } else { (micros / 1e6 - WEBKIT_EPOCH_OFFSET).max(0.0) }
}

/// Every bookmark, bar first, in the browser's order.
pub(super) fn parse_bookmarks(json: &Value) -> Vec<ImportedBookmark> {
    fn walk(node: &Value, out: &mut Vec<ImportedBookmark>) {
        match node.get("type").and_then(Value::as_str) {
            Some("url") => {
                let url = node.get("url").and_then(Value::as_str).unwrap_or_default();
                if is_importable_url(url) {
                    let added = node
                        .get("date_added")
                        .and_then(Value::as_str)
                        .and_then(|s| s.parse::<f64>().ok())
                        .map(webkit_to_unix)
                        .unwrap_or(0.0);
                    out.push(ImportedBookmark {
                        title: node.get("name").and_then(Value::as_str).unwrap_or_default().to_string(),
                        url: url.to_string(),
                        added,
                    });
                }
            }
            _ => {
                for child in node.get("children").and_then(Value::as_array).into_iter().flatten() {
                    walk(child, out);
                }
            }
        }
    }
    let mut out = Vec::new();
    for root in ["bookmark_bar", "other", "synced"] {
        if let Some(node) = json.pointer(&format!("/roots/{root}")) {
            walk(node, &mut out);
        }
    }
    out
}

fn read_history(conn: &rusqlite::Connection) -> anyhow::Result<Vec<ImportedVisit>> {
    let mut stmt = conn.prepare(
        "SELECT url, title, visit_count, last_visit_time FROM urls \
         WHERE hidden = 0 AND last_visit_time > 0 ORDER BY last_visit_time DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map([HISTORY_LIMIT as i64], |r| {
        Ok(ImportedVisit {
            url: r.get::<_, String>(0)?,
            title: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
            visit_count: r.get::<_, i64>(2)?.clamp(1, u32::MAX as i64) as u32,
            last_visited: webkit_to_unix(r.get::<_, i64>(3)? as f64),
        })
    })?;
    Ok(rows.filter_map(Result::ok).filter(|v| is_importable_url(&v.url)).collect())
}

/// The newest session file: `Sessions/Session_<time>` (Chrome 85+), else
/// the older `Current Session`.
fn latest_session(profile: &Path) -> Option<PathBuf> {
    let newest = std::fs::read_dir(profile.join("Sessions"))
        .ok()?
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let stamp: u64 = name.strip_prefix("Session_")?.parse().ok()?;
            Some((stamp, e.path()))
        })
        .max_by_key(|(stamp, _)| *stamp)
        .map(|(_, p)| p);
    newest.or_else(|| Some(profile.join("Current Session")).filter(|p| p.is_file()))
}

/// The open tabs of one window, in tab strip order.
#[derive(Debug, Default, PartialEq)]
pub(super) struct SessionWindow {
    pub pinned: Vec<ImportedTab>,
    pub tabs: Vec<ImportedTab>,
}

#[derive(Default)]
struct SessionTab {
    window: Option<i32>,
    index: i32,
    pinned: bool,
    selected_nav: Option<i32>,
    navs: BTreeMap<i32, (String, String)>,
}

impl SessionTab {
    /// The navigation the tab was showing.
    fn current(&self) -> Option<&(String, String)> {
        self.selected_nav
            .and_then(|i| self.navs.get(&i))
            .or_else(|| self.navs.values().next_back())
    }

    fn shift_navs(&mut self, from: i32, by: i32) {
        let navs = std::mem::take(&mut self.navs);
        self.navs = navs
            .into_iter()
            .map(|(i, n)| if i >= from { (i.saturating_sub(by), n) } else { (i, n) })
            .collect();
        if let Some(s) = self.selected_nav.as_mut()
            && *s >= from
        {
            *s = s.saturating_sub(by);
        }
    }
}

// Command ids of Chromium's session service (session_service_commands.cc).
const SET_TAB_WINDOW: u8 = 0;
const SET_TAB_INDEX_IN_WINDOW: u8 = 2;
const PRUNED_FROM_BACK: u8 = 5;
const UPDATE_TAB_NAVIGATION: u8 = 6;
const SET_SELECTED_NAVIGATION_INDEX: u8 = 7;
const PRUNED_FROM_FRONT: u8 = 11;
const SET_PINNED_STATE: u8 = 12;
const TAB_CLOSED: u8 = 16;
const WINDOW_CLOSED: u8 = 17;
const PRUNED: u8 = 24;

/// Replay an SNSS session log into its open windows (windows without a
/// web page are left out).
pub(super) fn parse_session(bytes: &[u8]) -> anyhow::Result<Vec<SessionWindow>> {
    if bytes.len() < 8 || &bytes[..4] != b"SNSS" {
        bail!("not a session file");
    }
    let mut tabs: HashMap<i32, SessionTab> = HashMap::new();
    let mut window_order: Vec<i32> = Vec::new();
    let mut closed_windows: Vec<i32> = Vec::new();
    let mut pos = 8;
    while pos + 3 <= bytes.len() {
        let size = u16::from_le_bytes([bytes[pos], bytes[pos + 1]]) as usize;
        if size == 0 || pos + 2 + size > bytes.len() {
            break;
        }
        let id = bytes[pos + 2];
        let p = &bytes[pos + 3..pos + 2 + size];
        pos += 2 + size;
        let int = |at: usize| p.get(at..at + 4).map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        match id {
            SET_TAB_WINDOW => {
                if let (Some(window), Some(tab)) = (int(0), int(4)) {
                    tabs.entry(tab).or_default().window = Some(window);
                    if !window_order.contains(&window) {
                        window_order.push(window);
                    }
                }
            }
            SET_TAB_INDEX_IN_WINDOW => {
                if let (Some(tab), Some(index)) = (int(0), int(4)) {
                    tabs.entry(tab).or_default().index = index;
                }
            }
            UPDATE_TAB_NAVIGATION => {
                if let Some((tab, index, url, title)) = parse_navigation(p) {
                    tabs.entry(tab).or_default().navs.insert(index, (url, title));
                }
            }
            SET_SELECTED_NAVIGATION_INDEX => {
                if let (Some(tab), Some(index)) = (int(0), int(4)) {
                    tabs.entry(tab).or_default().selected_nav = Some(index);
                }
            }
            SET_PINNED_STATE => {
                if let Some(tab) = int(0) {
                    tabs.entry(tab).or_default().pinned = p.get(4).is_some_and(|b| *b != 0);
                }
            }
            PRUNED_FROM_BACK => {
                if let (Some(tab), Some(count)) = (int(0), int(4))
                    && let Some(t) = tabs.get_mut(&tab)
                {
                    t.navs.retain(|i, _| *i < count);
                }
            }
            PRUNED_FROM_FRONT => {
                if let (Some(tab), Some(count)) = (int(0), int(4))
                    && let Some(t) = tabs.get_mut(&tab)
                {
                    t.navs.retain(|i, _| *i >= count);
                    t.shift_navs(count, count);
                }
            }
            PRUNED => {
                if let (Some(tab), Some(index), Some(count)) = (int(0), int(4), int(8))
                    && let Some(t) = tabs.get_mut(&tab)
                {
                    let end = index.saturating_add(count);
                    t.navs.retain(|i, _| *i < index || *i >= end);
                    t.shift_navs(end, count);
                }
            }
            TAB_CLOSED => {
                if let Some(tab) = int(0) {
                    tabs.remove(&tab);
                }
            }
            WINDOW_CLOSED => {
                if let Some(window) = int(0) {
                    closed_windows.push(window);
                }
            }
            _ => {}
        }
    }
    let mut windows = Vec::new();
    for window in window_order.into_iter().filter(|w| !closed_windows.contains(w)) {
        let mut members: Vec<&SessionTab> = tabs.values().filter(|t| t.window == Some(window)).collect();
        members.sort_by_key(|t| t.index);
        let mut w = SessionWindow::default();
        for t in members {
            let Some((url, title)) = t.current() else { continue };
            if !is_importable_url(url) {
                continue;
            }
            let tab = ImportedTab::new(url.clone(), title.clone());
            if t.pinned { w.pinned.push(tab) } else { w.tabs.push(tab) }
        }
        if !(w.pinned.is_empty() && w.tabs.is_empty()) {
            windows.push(w);
        }
    }
    Ok(windows)
}

/// `(tab id, navigation index, url, title)` of an UpdateTabNavigation
/// pickle: a u32 payload size, then 4-byte aligned fields.
fn parse_navigation(p: &[u8]) -> Option<(i32, i32, String, String)> {
    let mut r = PickleReader { data: p.get(4..)?, pos: 0 };
    let tab = r.int()?;
    let index = r.int()?;
    let url = r.string()?;
    let title = r.string16()?;
    Some((tab, index, url, title))
}

struct PickleReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl PickleReader<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        let out = self.data.get(self.pos..self.pos + n)?;
        // Every field is padded to a multiple of four bytes.
        self.pos += n.div_ceil(4) * 4;
        Some(out)
    }

    fn int(&mut self) -> Option<i32> {
        self.take(4).map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn string(&mut self) -> Option<String> {
        let len = usize::try_from(self.int()?).ok()?;
        self.take(len).map(|b| String::from_utf8_lossy(b).into_owned())
    }

    fn string16(&mut self) -> Option<String> {
        let len = usize::try_from(self.int()?).ok()?;
        let bytes = self.take(len.checked_mul(2)?)?;
        let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
        Some(String::from_utf16_lossy(&units))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::test_support::Tree;

    /// Builds SNSS logs the way Chromium writes them.
    #[derive(Default)]
    struct Snss(Vec<u8>);

    impl Snss {
        fn new() -> Snss {
            let mut v = b"SNSS".to_vec();
            v.extend(3i32.to_le_bytes());
            Snss(v)
        }

        fn command(&mut self, id: u8, payload: &[u8]) -> &mut Self {
            self.0.extend(((payload.len() + 1) as u16).to_le_bytes());
            self.0.push(id);
            self.0.extend(payload);
            self
        }

        fn ints(&mut self, id: u8, ints: &[i32]) -> &mut Self {
            let payload: Vec<u8> = ints.iter().flat_map(|i| i.to_le_bytes()).collect();
            self.command(id, &payload)
        }

        fn nav(&mut self, tab: i32, index: i32, url: &str, title: &str) -> &mut Self {
            fn pad(v: &mut Vec<u8>) {
                while v.len() % 4 != 0 {
                    v.push(0);
                }
            }
            let mut body = Vec::new();
            body.extend(tab.to_le_bytes());
            body.extend(index.to_le_bytes());
            body.extend((url.len() as i32).to_le_bytes());
            body.extend(url.as_bytes());
            pad(&mut body);
            let units: Vec<u16> = title.encode_utf16().collect();
            body.extend((units.len() as i32).to_le_bytes());
            for u in units {
                body.extend(u.to_le_bytes());
            }
            pad(&mut body);
            // Trailing fields Chromium writes (page state, transition…).
            body.extend(0i32.to_le_bytes());
            let mut payload = (body.len() as u32).to_le_bytes().to_vec();
            payload.extend(body);
            self.command(UPDATE_TAB_NAVIGATION, &payload)
        }
    }

    #[test]
    fn damaged_prune_counts_do_not_overflow() {
        let mut s = Snss::new();
        s.ints(SET_TAB_WINDOW, &[1, 10])
            .nav(10, 0, "https://a.test/", "A")
            .ints(SET_SELECTED_NAVIGATION_INDEX, &[10, i32::MAX])
            .ints(PRUNED, &[10, i32::MAX, i32::MAX])
            .ints(PRUNED_FROM_FRONT, &[10, i32::MIN])
            .ints(PRUNED, &[10, i32::MIN, i32::MIN]);
        // No panic; the tab survives with whatever is left.
        let windows = parse_session(&s.0).unwrap();
        assert!(windows.len() <= 1);
    }

    #[test]
    fn session_log_replays_to_open_tabs() {
        let mut s = Snss::new();
        s.ints(SET_TAB_WINDOW, &[1, 10])
            .ints(SET_TAB_INDEX_IN_WINDOW, &[10, 1])
            .nav(10, 0, "https://a.test/", "A")
            .nav(10, 1, "https://a.test/next", "A next")
            .ints(SET_SELECTED_NAVIGATION_INDEX, &[10, 0])
            .ints(SET_TAB_WINDOW, &[1, 11])
            .ints(SET_TAB_INDEX_IN_WINDOW, &[11, 0])
            .nav(11, 0, "https://pinned.test/", "Pinnéd ✓")
            .command(SET_PINNED_STATE, &[11, 0, 0, 0, 1, 0, 0, 0])
            .ints(SET_TAB_WINDOW, &[1, 12])
            .nav(12, 0, "chrome://newtab/", "New Tab")
            .ints(SET_TAB_WINDOW, &[1, 13])
            .ints(SET_TAB_INDEX_IN_WINDOW, &[13, 2])
            .nav(13, 0, "https://closed.test/", "Gone")
            .ints(TAB_CLOSED, &[13, 0, 0, 0])
            .ints(SET_TAB_WINDOW, &[2, 20])
            .nav(20, 0, "https://other-window.test/", "W2")
            .ints(SET_TAB_WINDOW, &[3, 30])
            .nav(30, 0, "https://closed-window.test/", "W3")
            .ints(WINDOW_CLOSED, &[3, 0, 0, 0]);
        let windows = parse_session(&s.0).unwrap();
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].pinned, vec![ImportedTab::new("https://pinned.test/", "Pinnéd ✓")]);
        assert_eq!(windows[0].tabs, vec![ImportedTab::new("https://a.test/", "A")]);
        assert_eq!(windows[1].tabs[0].url, "https://other-window.test/");
    }

    #[test]
    fn pruning_shifts_navigation_indices() {
        let mut s = Snss::new();
        s.ints(SET_TAB_WINDOW, &[1, 1])
            .nav(1, 0, "https://0.test/", "")
            .nav(1, 1, "https://1.test/", "")
            .nav(1, 2, "https://2.test/", "")
            .ints(SET_SELECTED_NAVIGATION_INDEX, &[1, 2])
            .ints(PRUNED_FROM_FRONT, &[1, 1]);
        assert_eq!(parse_session(&s.0).unwrap()[0].tabs[0].url, "https://2.test/");
        s.ints(PRUNED, &[1, 0, 1]);
        assert_eq!(parse_session(&s.0).unwrap()[0].tabs[0].url, "https://2.test/");
        s.ints(PRUNED_FROM_BACK, &[1, 0]);
        assert!(parse_session(&s.0).unwrap().is_empty());
    }

    #[test]
    fn truncated_or_foreign_files() {
        assert!(parse_session(b"nope").is_err());
        let mut s = Snss::new();
        s.ints(SET_TAB_WINDOW, &[1, 1]).nav(1, 0, "https://a.test/", "A");
        let mut cut = s.0.clone();
        cut.truncate(cut.len() - 3);
        // The incomplete last command is ignored, not misread.
        assert!(parse_session(&cut).unwrap().is_empty());
    }

    #[test]
    fn bookmarks_walk_every_root() {
        let json: Value = serde_json::from_str(
            r#"{"roots":{
                "bookmark_bar":{"type":"folder","children":[
                    {"type":"url","name":"Rust","url":"https://rust-lang.org/","date_added":"13436651453816365"},
                    {"type":"folder","name":"Dev","children":[{"type":"url","name":"Docs","url":"https://docs.rs/"}]}
                ]},
                "other":{"type":"folder","children":[{"type":"url","name":"Flags","url":"chrome://flags"}]},
                "synced":{"type":"folder","children":[{"type":"url","name":"","url":"https://m.test/"}]}
            }}"#,
        )
        .unwrap();
        let b = parse_bookmarks(&json);
        assert_eq!(b.iter().map(|b| b.url.as_str()).collect::<Vec<_>>(), [
            "https://rust-lang.org/",
            "https://docs.rs/",
            "https://m.test/"
        ]);
        // 13436651453816365 µs after 1601 is 2026-10-16.
        assert!((b[0].added - 1_792_177_853.8).abs() < 1.0, "{}", b[0].added);
        assert_eq!(b[1].added, 0.0);
    }

    #[test]
    fn history_reads_a_copy_newest_first() {
        let mut tree = Tree::new();
        let path = tree.path("History");
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE urls(id INTEGER PRIMARY KEY, url TEXT, title TEXT, visit_count INTEGER, \
                 typed_count INTEGER, last_visit_time INTEGER, hidden INTEGER);
                 INSERT INTO urls VALUES(1,'https://old.test/','Old',3,0,13000000000000000,0);
                 INSERT INTO urls VALUES(2,'https://new.test/',NULL,1,0,13436651453816365,0);
                 INSERT INTO urls VALUES(3,'https://hidden.test/','H',1,0,13436651453816366,1);
                 INSERT INTO urls VALUES(4,'chrome://history/','H',1,0,13436651453816367,0);",
            )
            .unwrap();
        }
        tree.track(&path);
        let db = DbCopy::open(&path).unwrap();
        let visits = read_history(db.conn()).unwrap();
        assert_eq!(visits.iter().map(|v| v.url.as_str()).collect::<Vec<_>>(), [
            "https://new.test/",
            "https://old.test/"
        ]);
        assert_eq!(visits[0].title, "");
        assert_eq!(visits[1].visit_count, 3);
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn profiles_follow_local_state() {
        let mut tree = Tree::new();
        tree.write(
            ".config/google-chrome/Local State",
            r#"{"profile":{"info_cache":{"Default":{"name":"Personal"},"Profile 1":{"name":"Work"},"Profile 9":{"name":"Gone"}},
                "profiles_order":["Profile 1","Default"]}}"#,
        );
        tree.write(".config/google-chrome/Default/Bookmarks", "{}");
        tree.write(".config/google-chrome/Profile 1/History", "");
        tree.write(".config/google-chrome/Profile 1/Sessions/Session_13436651453816365", "SNSS");
        tree.write(".config/google-chrome/Profile 1/Sessions/Session_13436575920023040", "SNSS");
        tree.write(".config/chromium/Default/History", "");
        let found = detect(&tree.root);
        let names: Vec<(BrowserKind, &str)> = found.iter().map(|s| (s.kind, s.profile.as_str())).collect();
        assert_eq!(names, [
            (BrowserKind::Chrome, "Work"),
            (BrowserKind::Chrome, "Personal"),
            (BrowserKind::Chromium, "Default")
        ]);
        assert_eq!(found[0].contents, Contents { bookmarks: false, history: true, tabs: true });
        assert_eq!(found[1].contents, Contents { bookmarks: true, history: false, tabs: false });
        assert!(
            latest_session(&found[0].path)
                .unwrap()
                .ends_with("Session_13436651453816365")
        );
    }
}
