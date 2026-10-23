//! Arc's sidebar (`StorableSidebar.json`): spaces with their pinned tabs
//! and folders, today's tabs, and the favorites grid ("top apps").
//!
//! Arc keeps no bookmarks or history of its own here. On macOS the file is
//! read where Arc keeps it; Arc has no Linux build, so on Linux a copy of
//! the file placed in the home or Downloads folder is picked up.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use serde_json::Value;

use super::{
    BrowserKind, Contents, Imported, ImportedFolder, ImportedSpace, ImportedTab, Source, Step, is_importable_url,
    read_json_file,
};

pub const SIDEBAR_FILE: &str = "StorableSidebar.json";

fn candidates(home: &Path) -> Vec<PathBuf> {
    let mut out = vec![home.join("Library/Application Support/Arc").join(SIDEBAR_FILE)];
    out.push(home.join(SIDEBAR_FILE));
    out.push(dirs::download_dir().unwrap_or_else(|| home.join("Downloads")).join(SIDEBAR_FILE));
    out.push(home.join("Downloads").join(SIDEBAR_FILE));
    out.dedup();
    out
}

pub(super) fn detect(home: &Path) -> Vec<Source> {
    candidates(home)
        .into_iter()
        .find(|p| p.is_file())
        .map(|path| Source {
            kind: BrowserKind::Arc,
            profile: "Sidebar".into(),
            path,
            contents: Contents {
                bookmarks: false,
                history: false,
                cookies: false,
                tabs: true,
            },
        })
        .into_iter()
        .collect()
}

pub(super) fn read(source: &Source, step: Step, out: &mut Imported) -> anyhow::Result<()> {
    if step == Step::Tabs {
        let json = read_json_file(&source.path)?;
        let (favorites, spaces) = parse_sidebar(&json);
        out.favorites = favorites;
        out.spaces = spaces;
    }
    Ok(())
}

/// Objects in one of Arc's flattened `[id, object, id, object…]` lists.
fn objects(list: Option<&Value>) -> impl Iterator<Item = &Value> {
    list.and_then(Value::as_array).into_iter().flatten().filter(|v| v.is_object())
}

/// Ids (strings) in a list that interleaves them with marker objects.
fn ids(list: Option<&Value>) -> Vec<&str> {
    list.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect()
}

pub(super) fn parse_sidebar(json: &Value) -> (Vec<ImportedTab>, Vec<ImportedSpace>) {
    let containers = json.pointer("/sidebar/containers").and_then(Value::as_array);
    let Some(container) = containers.into_iter().flatten().find(|c| c.get("items").is_some()) else {
        return (Vec::new(), Vec::new());
    };
    let items: HashMap<&str, &Value> = objects(container.get("items"))
        .filter_map(|i| Some((i.get("id")?.as_str()?, i)))
        .collect();
    let children = |id: &str| -> Vec<&Value> {
        let Some(item) = items.get(id) else { return Vec::new() };
        ids(item.get("childrenIds")).into_iter().filter_map(|c| items.get(c).copied()).collect()
    };

    let favorites: Vec<ImportedTab> = ids(container.get("topAppsContainerIDs"))
        .into_iter()
        .flat_map(&children)
        .filter_map(tab_of)
        .collect();

    let mut spaces = Vec::new();
    for space in objects(container.get("spaces")) {
        let container_ids = ids(space.get("containerIDs"));
        // `["unpinned", <id>, "pinned", <id>]`: each label precedes its id.
        let after = |label: &str| {
            container_ids
                .iter()
                .position(|v| *v == label)
                .and_then(|i| container_ids.get(i + 1).copied())
        };
        let mut out = ImportedSpace {
            name: space.get("title").and_then(Value::as_str).unwrap_or_default().to_string(),
            ..ImportedSpace::default()
        };
        if let Some(pinned) = after("pinned") {
            for item in children(pinned) {
                match tab_of(item) {
                    Some(tab) => out.pinned.push(tab),
                    None if item.pointer("/data/list").is_some() => {
                        // Mori folders don't nest: a folder takes the tabs
                        // of its subfolders.
                        let mut folder = ImportedFolder {
                            name: item.get("title").and_then(Value::as_str).unwrap_or("Folder").to_string(),
                            tabs: Vec::new(),
                        };
                        collect_tabs(item, &children, &mut folder.tabs, 0);
                        if !folder.tabs.is_empty() {
                            out.folders.push(folder);
                        }
                    }
                    None => {}
                }
            }
        }
        if let Some(unpinned) = after("unpinned") {
            let mut tabs = Vec::new();
            for item in children(unpinned) {
                match tab_of(item) {
                    Some(tab) => tabs.push(tab),
                    None => collect_tabs(item, &children, &mut tabs, 0),
                }
            }
            out.tabs = tabs;
        }
        spaces.push(out);
    }
    (favorites, spaces)
}

fn collect_tabs<'a>(
    item: &'a Value,
    children: &dyn Fn(&str) -> Vec<&'a Value>,
    out: &mut Vec<ImportedTab>,
    depth: usize,
) {
    if depth > 32 {
        return;
    }
    let Some(id) = item.get("id").and_then(Value::as_str) else { return };
    for child in children(id) {
        match tab_of(child) {
            Some(tab) => out.push(tab),
            None => collect_tabs(child, children, out, depth + 1),
        }
    }
}

/// A tab item's page; its custom title wins over the saved page title.
fn tab_of(item: &Value) -> Option<ImportedTab> {
    let tab = item.pointer("/data/tab")?;
    let url = tab.get("savedURL").and_then(Value::as_str)?;
    if !is_importable_url(url) {
        return None;
    }
    let title = item
        .get("title")
        .and_then(Value::as_str)
        .filter(|t| !t.trim().is_empty())
        .or_else(|| tab.get("savedTitle").and_then(Value::as_str))
        .unwrap_or_default();
    Some(ImportedTab::new(url, title))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(id: &str, parent: &str, url: &str, title: Option<&str>) -> Value {
        serde_json::json!({ "id": id, "parentID": parent, "childrenIds": [], "title": title,
            "data": { "tab": { "savedURL": url, "savedTitle": format!("Saved {id}") } } })
    }

    fn list(id: &str, title: &str, children: &[&str]) -> Value {
        serde_json::json!({ "id": id, "title": title, "childrenIds": children, "data": { "list": {} } })
    }

    fn container(id: &str, children: &[&str]) -> Value {
        serde_json::json!({ "id": id, "title": null, "childrenIds": children,
            "data": { "itemContainer": { "containerType": { "spaceItems": { "_0": "S1" } } } } })
    }

    #[test]
    fn spaces_pins_folders_and_favorites() {
        let json = serde_json::json!({ "sidebar": { "containers": [
            { "global": {} },
            {
                "spaces": ["S1", { "id": "S1", "title": "Personal",
                                   "containerIDs": ["unpinned", "U1", "pinned", "P1"] }],
                "topAppsContainerIDs": [{ "default": true }, "TOP"],
                "items": [
                    "TOP", container("TOP", &["f1"]),
                    "f1", tab("f1", "TOP", "https://mail.test/", None),
                    "P1", container("P1", &["p1", "L1", "x"]),
                    "p1", tab("p1", "P1", "https://docs.test/", Some("My Docs")),
                    "L1", list("L1", "Reading", &["l1", "L2"]),
                    "l1", tab("l1", "L1", "https://read.test/", None),
                    "L2", list("L2", "Deeper", &["l2"]),
                    "l2", tab("l2", "L2", "https://deep.test/", None),
                    "U1", container("U1", &["u1", "u2"]),
                    "u1", tab("u1", "U1", "https://today.test/", None),
                    "u2", tab("u2", "U1", "arc://settings", None)
                ]
            }
        ]}});
        let (favorites, spaces) = parse_sidebar(&json);
        assert_eq!(favorites, vec![ImportedTab::new("https://mail.test/", "Saved f1")]);
        assert_eq!(spaces.len(), 1);
        let s = &spaces[0];
        assert_eq!(s.name, "Personal");
        assert_eq!(s.pinned, vec![ImportedTab::new("https://docs.test/", "My Docs")]);
        assert_eq!(s.folders.len(), 1);
        assert_eq!(s.folders[0].name, "Reading");
        assert_eq!(
            s.folders[0].tabs.iter().map(|t| t.url.as_str()).collect::<Vec<_>>(),
            ["https://read.test/", "https://deep.test/"]
        );
        assert_eq!(s.tabs, vec![ImportedTab::new("https://today.test/", "Saved u1")]);
    }

    #[test]
    fn unexpected_shapes_yield_nothing() {
        assert_eq!(parse_sidebar(&serde_json::json!({})), (Vec::new(), Vec::new()));
        assert_eq!(
            parse_sidebar(&serde_json::json!({ "sidebar": { "containers": [{ "global": {} }] } })),
            (Vec::new(), Vec::new())
        );
    }
}
