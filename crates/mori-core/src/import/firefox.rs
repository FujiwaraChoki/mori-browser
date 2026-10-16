//! Firefox-family profiles (Firefox, Zen): `places.sqlite` for bookmarks
//! and history, and the session (`sessionstore` / Zen's `zen-sessions`,
//! both mozLz4-compressed JSON) for open tabs, Zen workspaces, folders,
//! pinned tabs and essentials.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use anyhow::{Context as _, bail};
use serde_json::Value;

use super::{
    BrowserKind, Contents, DbCopy, HISTORY_LIMIT, Imported, ImportedBookmark, ImportedFolder, ImportedSpace,
    ImportedTab, ImportedVisit, Source, Step, is_importable_url,
};

/// Profile roots (holding `profiles.ini`) per browser, relative to home.
fn roots() -> &'static [(BrowserKind, &'static str)] {
    if cfg!(target_os = "macos") {
        &[
            (BrowserKind::Firefox, "Library/Application Support/Firefox"),
            (BrowserKind::Zen, "Library/Application Support/zen"),
        ]
    } else {
        &[
            (BrowserKind::Firefox, ".mozilla/firefox"),
            (BrowserKind::Firefox, ".config/mozilla/firefox"),
            (BrowserKind::Firefox, ".var/app/org.mozilla.firefox/.mozilla/firefox"),
            (BrowserKind::Firefox, "snap/firefox/common/.mozilla/firefox"),
            (BrowserKind::Zen, ".config/zen"),
            (BrowserKind::Zen, ".zen"),
            (BrowserKind::Zen, ".var/app/app.zen_browser.zen/.zen"),
        ]
    }
}

pub(super) fn detect(home: &Path) -> Vec<Source> {
    let mut out = Vec::new();
    for (kind, rel) in roots() {
        let root = home.join(rel);
        let Ok(ini) = std::fs::read_to_string(root.join("profiles.ini")) else { continue };
        for (name, path) in profiles(&root, &ini) {
            let places = path.join("places.sqlite").is_file();
            let contents = Contents {
                bookmarks: places,
                history: places,
                tabs: session_file(&path).is_some(),
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

/// `(name, directory)` of each profile in a `profiles.ini`, the profile an
/// installation uses first.
fn profiles(root: &Path, ini: &str) -> Vec<(String, PathBuf)> {
    let sections = parse_ini(ini);
    let in_use: Vec<&str> = sections
        .iter()
        .filter(|(name, _)| !name.starts_with("Profile") && name != "General")
        .filter_map(|(_, keys)| keys.get("Default").map(String::as_str))
        .collect();
    let mut out: Vec<(bool, String, PathBuf)> = sections
        .iter()
        .filter(|(name, _)| name.starts_with("Profile"))
        .filter_map(|(_, keys)| {
            let rel = keys.get("Path")?;
            let relative = keys.get("IsRelative").is_none_or(|v| v == "1");
            let path = if relative { root.join(rel) } else { PathBuf::from(rel) };
            let name = keys.get("Name").cloned().unwrap_or_else(|| rel.clone());
            Some((in_use.contains(&rel.as_str()), name, path))
        })
        .filter(|(_, _, path)| path.is_dir())
        .collect();
    // Stable: the in-use profile first, the rest in file order.
    out.sort_by_key(|(used, _, _)| !*used);
    out.into_iter().map(|(_, name, path)| (name, path)).collect()
}

/// `[section]` → `key=value` pairs, in file order.
fn parse_ini(text: &str) -> Vec<(String, HashMap<String, String>)> {
    let mut out: Vec<(String, HashMap<String, String>)> = Vec::new();
    for line in text.lines().map(str::trim) {
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            out.push((name.to_string(), HashMap::new()));
        } else if let Some((k, v)) = line.split_once('=')
            && let Some((_, keys)) = out.last_mut()
        {
            keys.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    out
}

/// The freshest session: Zen's own file, else Firefox's running session,
/// its clean-shutdown copy, then the previous one.
fn session_file(profile: &Path) -> Option<PathBuf> {
    [
        "zen-sessions.jsonlz4",
        "sessionstore-backups/recovery.jsonlz4",
        "sessionstore.jsonlz4",
        "sessionstore-backups/previous.jsonlz4",
    ]
    .iter()
    .map(|rel| profile.join(rel))
    .find(|p| p.is_file())
}

pub(super) fn read(source: &Source, step: Step, out: &mut Imported) -> anyhow::Result<()> {
    match step {
        Step::Bookmarks => {
            let db = DbCopy::open(&source.path.join("places.sqlite"))?;
            out.bookmarks = read_bookmarks(db.conn())?;
        }
        Step::History => {
            let db = DbCopy::open(&source.path.join("places.sqlite"))?;
            out.history = read_history(db.conn())?;
        }
        Step::Tabs => {
            let path = session_file(&source.path).context("no session file")?;
            let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
            let json: Value = serde_json::from_slice(&decompress_mozlz4(&bytes)?).context("parsing session")?;
            let (favorites, spaces) = parse_session(&json);
            out.favorites = favorites;
            out.spaces = spaces;
        }
    }
    Ok(())
}

/// Decode Mozilla's `mozLz40\0` container: an 8-byte magic, the decoded
/// size (u32 LE) and one LZ4 block.
pub(super) fn decompress_mozlz4(bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    const MAGIC: &[u8] = b"mozLz40\0";
    if bytes.len() < 12 || &bytes[..8] != MAGIC {
        bail!("not a mozLz4 file");
    }
    let size = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
    lz4_flex::block::decompress(&bytes[12..], size).map_err(|e| anyhow::anyhow!("lz4: {e}"))
}

/// Bookmarks in the order Firefox shows them: toolbar, menu, other, mobile.
fn read_bookmarks(conn: &rusqlite::Connection) -> anyhow::Result<Vec<ImportedBookmark>> {
    struct Row {
        parent: i64,
        kind: i64,
        position: i64,
        title: String,
        url: Option<String>,
        added: i64,
        guid: String,
    }
    let mut stmt = conn.prepare(
        "SELECT b.id, b.parent, b.type, b.position, b.title, p.url, b.dateAdded, b.guid \
         FROM moz_bookmarks b LEFT JOIN moz_places p ON b.fk = p.id",
    )?;
    let rows: HashMap<i64, Row> = stmt
        .query_map([], |r| {
            Ok((r.get::<_, i64>(0)?, Row {
                parent: r.get::<_, Option<i64>>(1)?.unwrap_or(0),
                kind: r.get::<_, i64>(2)?,
                position: r.get::<_, Option<i64>>(3)?.unwrap_or(0),
                title: r.get::<_, Option<String>>(4)?.unwrap_or_default(),
                url: r.get::<_, Option<String>>(5)?,
                added: r.get::<_, Option<i64>>(6)?.unwrap_or(0),
                guid: r.get::<_, Option<String>>(7)?.unwrap_or_default(),
            }))
        })?
        .filter_map(Result::ok)
        .collect();
    let mut children: HashMap<i64, Vec<i64>> = HashMap::new();
    for (id, row) in &rows {
        children.entry(row.parent).or_default().push(*id);
    }
    for list in children.values_mut() {
        list.sort_by_key(|id| rows[id].position);
    }
    fn walk(
        id: i64,
        rows: &HashMap<i64, Row>,
        children: &HashMap<i64, Vec<i64>>,
        out: &mut Vec<ImportedBookmark>,
        depth: usize,
    ) {
        if depth > 64 {
            return;
        }
        for child in children.get(&id).into_iter().flatten() {
            let row = &rows[child];
            match (row.kind, &row.url) {
                (1, Some(url)) if is_importable_url(url) => out.push(ImportedBookmark {
                    title: row.title.clone(),
                    url: url.clone(),
                    added: row.added as f64 / 1e6,
                }),
                (2, _) => walk(*child, rows, children, out, depth + 1),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    for guid in ["toolbar_____", "menu________", "unfiled_____", "mobile______"] {
        if let Some((id, _)) = rows.iter().find(|(_, r)| r.guid == guid) {
            walk(*id, &rows, &children, &mut out, 0);
        }
    }
    Ok(out)
}

fn read_history(conn: &rusqlite::Connection) -> anyhow::Result<Vec<ImportedVisit>> {
    let mut stmt = conn.prepare(
        "SELECT url, title, visit_count, last_visit_date FROM moz_places \
         WHERE hidden = 0 AND visit_count > 0 AND last_visit_date IS NOT NULL \
         ORDER BY last_visit_date DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map([HISTORY_LIMIT as i64], |r| {
        Ok(ImportedVisit {
            url: r.get::<_, String>(0)?,
            title: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
            visit_count: r.get::<_, i64>(2)?.clamp(1, u32::MAX as i64) as u32,
            last_visited: r.get::<_, i64>(3)? as f64 / 1e6,
        })
    })?;
    Ok(rows.filter_map(Result::ok).filter(|v| is_importable_url(&v.url)).collect())
}

/// Favorites (Zen essentials) and spaces from a session: Zen's
/// `zen-sessions` (tabs, folders and spaces at the top level) or a
/// `sessionstore` (one object per window, Zen keeps the same keys there).
pub(super) fn parse_session(json: &Value) -> (Vec<ImportedTab>, Vec<ImportedSpace>) {
    let windows: Vec<&Value> = match json.get("windows").and_then(Value::as_array) {
        Some(windows) => windows.iter().collect(),
        None => vec![json],
    };
    let zen = windows.iter().any(|w| {
        w.get("spaces").is_some() || tabs_of(w).any(|t| t.get("zenWorkspace").and_then(Value::as_str).is_some())
    });
    if zen {
        parse_zen(&windows)
    } else {
        let spaces = windows
            .iter()
            .enumerate()
            .map(|(i, w)| parse_firefox_window(w, i))
            .collect();
        (Vec::new(), spaces)
    }
}

fn tabs_of(window: &Value) -> impl Iterator<Item = &Value> {
    window.get("tabs").and_then(Value::as_array).into_iter().flatten()
}

fn str_field<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

fn flag(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}

/// The page a session tab shows (`index` is 1-based into `entries`). For
/// pinned tabs Zen remembers the page they were pinned at, which is where
/// they reset to, so that one is used.
fn session_tab(tab: &Value) -> Option<ImportedTab> {
    let pinned_home = tab.pointer("/_zenPinnedInitialState/entry");
    let entry = pinned_home.filter(|e| str_field(e, "url").is_some()).or_else(|| {
        let entries = tab.get("entries")?.as_array()?;
        let index = tab.get("index").and_then(Value::as_u64).unwrap_or(entries.len() as u64);
        entries.get((index as usize).saturating_sub(1)).or_else(|| entries.last())
    })?;
    let url = str_field(entry, "url")?;
    if !is_importable_url(url) {
        return None;
    }
    let title = str_field(tab, "zenStaticLabel")
        .filter(|l| !l.trim().is_empty())
        .or_else(|| str_field(entry, "title"))
        .unwrap_or_default();
    Some(ImportedTab::new(url, title))
}

/// A Firefox window: pinned tabs, tab groups as folders, the rest loose.
/// Tabs hidden by extensions are left out.
fn parse_firefox_window(window: &Value, index: usize) -> ImportedSpace {
    let group_names: HashMap<&str, &str> = window
        .get("groups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|g| Some((str_field(g, "id")?, str_field(g, "name").unwrap_or_default())))
        .collect();
    let mut space = ImportedSpace {
        name: format!("Window {}", index + 1),
        ..ImportedSpace::default()
    };
    for tab in tabs_of(window).filter(|t| !flag(t, "hidden")) {
        let Some(imported) = session_tab(tab) else { continue };
        if flag(tab, "pinned") {
            space.pinned.push(imported);
        } else if let Some(group) = str_field(tab, "groupId") {
            push_to_folder(&mut space.folders, group_names.get(group).copied().unwrap_or("Group"), imported);
        } else {
            space.tabs.push(imported);
        }
    }
    space
}

fn push_to_folder(folders: &mut Vec<ImportedFolder>, name: &str, tab: ImportedTab) {
    match folders.iter_mut().find(|f| f.name == name) {
        Some(f) => f.tabs.push(tab),
        None => folders.push(ImportedFolder {
            name: name.to_string(),
            tabs: vec![tab],
        }),
    }
}

/// Zen: essentials become favorites; each workspace becomes a space with
/// its pinned tabs, folders and open tabs.
fn parse_zen(windows: &[&Value]) -> (Vec<ImportedTab>, Vec<ImportedSpace>) {
    let mut order: Vec<String> = Vec::new();
    let mut spaces: HashMap<String, ImportedSpace> = HashMap::new();
    let mut folder_names: HashMap<String, String> = HashMap::new();
    for w in windows {
        for s in w.get("spaces").and_then(Value::as_array).into_iter().flatten() {
            let Some(uuid) = str_field(s, "uuid") else { continue };
            if !spaces.contains_key(uuid) {
                order.push(uuid.to_string());
                spaces.insert(uuid.to_string(), ImportedSpace {
                    name: str_field(s, "name").unwrap_or_default().to_string(),
                    ..ImportedSpace::default()
                });
            }
        }
        for key in ["folders", "groups"] {
            for f in w.get(key).and_then(Value::as_array).into_iter().flatten() {
                if let (Some(id), Some(name)) = (str_field(f, "id"), str_field(f, "name")) {
                    folder_names.entry(id.to_string()).or_insert_with(|| name.to_string());
                }
            }
        }
    }
    let mut favorites = Vec::new();
    for tab in windows.iter().flat_map(|w| tabs_of(w)) {
        if flag(tab, "zenIsEmpty") || flag(tab, "zenIsGlance") {
            continue;
        }
        let Some(imported) = session_tab(tab) else { continue };
        if flag(tab, "zenEssential") {
            if !favorites.iter().any(|f: &ImportedTab| f.url == imported.url) {
                favorites.push(imported);
            }
            continue;
        }
        // Tabs of a workspace the file doesn't list go to the first one.
        let workspace = str_field(tab, "zenWorkspace")
            .filter(|w| spaces.contains_key(*w))
            .map(str::to_string)
            .or_else(|| order.first().cloned())
            .unwrap_or_default();
        if !spaces.contains_key(&workspace) {
            order.push(workspace.clone());
            spaces.insert(workspace.clone(), ImportedSpace::default());
        }
        let space = spaces.get_mut(&workspace).expect("inserted above");
        match str_field(tab, "groupId") {
            Some(group) => {
                let name = folder_names.get(group).map(String::as_str).unwrap_or("Folder");
                push_to_folder(&mut space.folders, name, imported);
            }
            None if flag(tab, "pinned") => space.pinned.push(imported),
            None => space.tabs.push(imported),
        }
    }
    let spaces = order.into_iter().filter_map(|id| spaces.remove(&id)).collect();
    (favorites, spaces)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::test_support::Tree;

    fn entry(url: &str, title: &str) -> Value {
        serde_json::json!({ "url": url, "title": title })
    }

    #[test]
    fn mozlz4_roundtrip() {
        let json = br#"{"windows":[]}"#;
        let mut file = b"mozLz40\0".to_vec();
        file.extend((json.len() as u32).to_le_bytes());
        file.extend(lz4_flex::block::compress(json));
        assert_eq!(decompress_mozlz4(&file).unwrap(), json);
        assert!(decompress_mozlz4(b"mozLz40\0").is_err());
        assert!(decompress_mozlz4(b"{\"windows\":[]}").is_err());
    }

    #[test]
    fn zen_workspaces_folders_pins_and_essentials() {
        // The shape of Zen's zen-sessions.jsonlz4.
        let json = serde_json::json!({
            "spaces": [
                { "uuid": "{a}", "name": "Space" },
                { "uuid": "{b}", "name": "Work" }
            ],
            "folders": [{ "id": "f1", "name": "Providers", "workspaceId": "{a}" }],
            "tabs": [
                { "entries": [entry("https://www.youtube.com/?x", "YouTube")], "index": 1,
                  "pinned": true, "zenEssential": true, "zenWorkspace": "{a}" },
                { "entries": [entry("https://mail.test/inbox/42", "Mail 42")], "index": 1,
                  "pinned": true, "zenWorkspace": "{a}",
                  "_zenPinnedInitialState": { "entry": entry("https://mail.test/", "Mail") } },
                { "entries": [entry("about:blank", "")], "index": 1, "pinned": true,
                  "zenIsEmpty": true, "groupId": "f1", "zenWorkspace": "{a}" },
                { "entries": [entry("https://openrouter.ai/", "OpenRouter")], "index": 1,
                  "pinned": true, "groupId": "f1", "zenWorkspace": "{a}" },
                { "entries": [entry("https://a.test/1", "One"), entry("https://a.test/2", "Two")], "index": 2,
                  "zenWorkspace": "{a}" },
                { "entries": [entry("https://glance.test/", "G")], "index": 1, "zenIsGlance": true,
                  "zenWorkspace": "{a}" },
                { "entries": [entry("https://work.test/", "W")], "index": 1, "zenWorkspace": "{b}",
                  "zenStaticLabel": "Renamed" },
                { "entries": [entry("https://orphan.test/", "O")], "index": 1, "zenWorkspace": "{gone}" }
            ]
        });
        let (favorites, spaces) = parse_session(&json);
        assert_eq!(favorites, vec![ImportedTab::new("https://www.youtube.com/?x", "YouTube")]);
        assert_eq!(spaces.len(), 2);
        let a = &spaces[0];
        assert_eq!(a.name, "Space");
        assert_eq!(a.pinned, vec![ImportedTab::new("https://mail.test/", "Mail")]);
        assert_eq!(a.folders, vec![ImportedFolder {
            name: "Providers".into(),
            tabs: vec![ImportedTab::new("https://openrouter.ai/", "OpenRouter")]
        }]);
        assert_eq!(a.tabs, vec![
            ImportedTab::new("https://a.test/2", "Two"),
            ImportedTab::new("https://orphan.test/", "O")
        ]);
        assert_eq!(spaces[1].tabs, vec![ImportedTab::new("https://work.test/", "Renamed")]);
    }

    #[test]
    fn firefox_windows_groups_and_hidden_tabs() {
        let json = serde_json::json!({ "windows": [
            { "tabs": [
                { "entries": [entry("https://p.test/", "P")], "index": 1, "pinned": true },
                { "entries": [entry("https://g.test/", "G")], "index": 1, "groupId": "g1" },
                { "entries": [entry("https://h.test/", "H")], "index": 1, "hidden": true },
                { "entries": [entry("https://l.test/", "L")] }
              ],
              "groups": [{ "id": "g1", "name": "Research" }] },
            { "tabs": [{ "entries": [entry("https://w2.test/", "W2")], "index": 1 }] }
        ]});
        let (favorites, spaces) = parse_session(&json);
        assert!(favorites.is_empty());
        assert_eq!(spaces.len(), 2);
        assert_eq!(spaces[0].name, "Window 1");
        assert_eq!(spaces[0].pinned[0].url, "https://p.test/");
        assert_eq!(spaces[0].folders[0].name, "Research");
        assert_eq!(spaces[0].tabs, vec![ImportedTab::new("https://l.test/", "L")]);
        assert_eq!(spaces[1].tabs[0].url, "https://w2.test/");
    }

    #[test]
    fn places_bookmarks_in_tree_order_and_history() {
        let mut tree = Tree::new();
        let path = tree.path("places.sqlite");
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch(
                "PRAGMA journal_mode=WAL;
                 CREATE TABLE moz_places(id INTEGER PRIMARY KEY, url TEXT, title TEXT, visit_count INTEGER,
                   hidden INTEGER, last_visit_date INTEGER);
                 CREATE TABLE moz_bookmarks(id INTEGER PRIMARY KEY, type INTEGER, fk INTEGER, parent INTEGER,
                   position INTEGER, title TEXT, dateAdded INTEGER, guid TEXT);
                 INSERT INTO moz_places VALUES(1,'https://b.test/','B',4,0,1700000000000000);
                 INSERT INTO moz_places VALUES(2,'https://a.test/','A',1,0,1790000000000000);
                 INSERT INTO moz_places VALUES(3,'place:sort=8','Recent',0,0,NULL);
                 INSERT INTO moz_places VALUES(4,'https://never.test/','N',0,0,NULL);
                 INSERT INTO moz_bookmarks VALUES(1,2,NULL,0,0,'',0,'root________');
                 INSERT INTO moz_bookmarks VALUES(2,2,NULL,1,0,'menu',0,'menu________');
                 INSERT INTO moz_bookmarks VALUES(3,2,NULL,1,1,'toolbar',0,'toolbar_____');
                 INSERT INTO moz_bookmarks VALUES(10,1,1,2,0,'In menu',1600000000000000,'x1');
                 INSERT INTO moz_bookmarks VALUES(11,2,NULL,3,1,'Sub',0,'x2');
                 INSERT INTO moz_bookmarks VALUES(12,1,2,11,0,'Deep',0,'x3');
                 INSERT INTO moz_bookmarks VALUES(13,1,3,3,0,'Smart',0,'x4');
                 INSERT INTO moz_bookmarks VALUES(14,1,4,3,2,'Last',0,'x5');",
            )
            .unwrap();
        }
        tree.track(&path);
        let db = DbCopy::open(&path).unwrap();
        let bookmarks = read_bookmarks(db.conn()).unwrap();
        // Toolbar first (its folder, then the later item), then the menu.
        assert_eq!(bookmarks.iter().map(|b| b.title.as_str()).collect::<Vec<_>>(), ["Deep", "Last", "In menu"]);
        assert_eq!(bookmarks[2].added, 1_600_000_000.0);
        let history = read_history(db.conn()).unwrap();
        assert_eq!(history.iter().map(|v| v.url.as_str()).collect::<Vec<_>>(), [
            "https://a.test/",
            "https://b.test/"
        ]);
        assert_eq!(history[1].visit_count, 4);
        assert_eq!(history[0].last_visited, 1_790_000_000.0);
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn profiles_ini_in_use_first() {
        let mut tree = Tree::new();
        tree.write(
            ".config/zen/profiles.ini",
            "[General]\nVersion=2\n\n[Profile0]\nName=Default (release)\nIsRelative=1\nPath=abc.Default (release)\n\n\
             [Install15B7]\nDefault=abc.Default (release)\n\n[Profile1]\nName=Default Profile\nIsRelative=1\n\
             Path=def.Default Profile\nDefault=1\n",
        );
        tree.write(".config/zen/def.Default Profile/times.json", "{}");
        tree.write(".config/zen/abc.Default (release)/places.sqlite", "");
        tree.write(".config/zen/abc.Default (release)/zen-sessions.jsonlz4", "");
        tree.write(".mozilla/firefox/profiles.ini", "[Profile0]\nName=default\nIsRelative=1\nPath=xyz.default\n");
        let found = detect(&tree.root);
        let names: Vec<(BrowserKind, &str)> = found.iter().map(|s| (s.kind, s.profile.as_str())).collect();
        // The Firefox profile directory doesn't exist; Zen's second profile is empty.
        assert_eq!(names, [(BrowserKind::Zen, "Default (release)"), (BrowserKind::Zen, "Default Profile")]);
        assert_eq!(found[0].contents, Contents { bookmarks: true, history: true, tabs: true });
        assert!(!found[1].contents.any());
    }
}
