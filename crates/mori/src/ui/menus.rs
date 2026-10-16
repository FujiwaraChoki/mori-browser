//! Context menus: page, link, image, media, tab, folder, space, sidebar
//! background, and "+".
//!
//! Styled like Zen's menus: a compact dark surface with small radius, 28px
//! rows, quiet icons, full-width separators and plain-text shortcuts. Menus
//! open at the cursor and flip to stay on screen without covering the point
//! they were opened from; button menus open beside their button.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use mori_core::{
    page_actions::{self, MediaAction},
    workspace::{FolderId, MAX_FAVORITES, MAX_FOLDER_DEPTH, Slot, Space, SpaceId, TabId, TabRecord},
};

use crate::{
    browser::{Browser, MenuKind, RenameTarget, ToastStyle},
    commands::{self, Command},
    theme::{ActiveTokens, metrics},
    ui::{
        components::{h_flex, icon, v_flex},
        launcher::LauncherMode,
        settings::SettingsCategory,
    },
};

pub(crate) type Handler = Rc<dyn Fn(&mut Browser, &mut Window, &mut Context<Browser>)>;
/// Draws a custom menu row; it may attach its own listeners.
pub(crate) type CustomRow = Rc<dyn Fn(&mut Window, &mut Context<Browser>) -> AnyElement>;

const MENU_WIDTH: f32 = 268.;
const MENU_PADDING: f32 = 4.;
const ITEM_HEIGHT: f32 = 28.;
const HEADER_HEIGHT: f32 = 26.;
const SEPARATOR_HEIGHT: f32 = 9.;
const ICON_CELL: f32 = 32.;
const ICON_COLUMNS: usize = 6;
const MENU_TEXT: Pixels = px(14.);
/// Gap between a menu and the button or point it was opened from.
const MENU_GAP: f32 = 4.;

/// Anchor key of the sidebar's "+" button (see [`anchor_probe`]).
pub const NEW_MENU_ANCHOR: &str = "new-menu";
/// Anchor key of the toolbar's "…" app-menu button.
pub const APP_MENU_ANCHOR: &str = "app-menu";

pub enum Entry {
    Item {
        label: SharedString,
        icon: Option<&'static str>,
        shortcut: Option<&'static str>,
        enabled: bool,
        destructive: bool,
        handler: Handler,
    },
    Header(SharedString),
    /// Muted explanatory text.
    Note(SharedString),
    Separator,
    /// A row with a switch; clicking it runs `handler` and keeps the menu open.
    Switch {
        label: SharedString,
        icon: &'static str,
        on: bool,
        handler: Handler,
    },
    /// A row drawn by the caller, `height` px tall; the menu stays open.
    Custom { height: f32, render: CustomRow },
    /// A folder's glyph picker, the current one marked.
    FolderIcons { folder: FolderId, current: String },
}

impl Entry {
    fn height(&self) -> f32 {
        match self {
            Entry::Separator => SEPARATOR_HEIGHT,
            Entry::Header(_) | Entry::Note(_) => HEADER_HEIGHT,
            Entry::Item { .. } | Entry::Switch { .. } => ITEM_HEIGHT,
            Entry::Custom { height, .. } => *height,
            Entry::FolderIcons { .. } => {
                let rows = crate::ui::sidebar::FOLDER_SYMBOLS.len().div_ceil(ICON_COLUMNS);
                rows as f32 * ICON_CELL + 8.
            }
        }
    }
}

pub(crate) fn item(
    label: impl Into<SharedString>,
    icon: &'static str,
    handler: impl Fn(&mut Browser, &mut Window, &mut Context<Browser>) + 'static,
) -> Entry {
    Entry::Item {
        label: label.into(),
        icon: Some(icon),
        shortcut: None,
        enabled: true,
        destructive: false,
        handler: Rc::new(handler),
    }
}

pub(crate) trait EntryExt {
    fn shortcut(self, keys: &'static str) -> Self;
    fn enabled(self, enabled: bool) -> Self;
    fn destructive(self) -> Self;
}

impl EntryExt for Entry {
    fn shortcut(mut self, keys: &'static str) -> Self {
        if let Entry::Item { shortcut, .. } = &mut self {
            *shortcut = Some(keys);
        }
        self
    }
    fn enabled(mut self, value: bool) -> Self {
        if let Entry::Item { enabled, .. } = &mut self {
            *enabled = value;
        }
        self
    }
    fn destructive(mut self) -> Self {
        if let Entry::Item { destructive, .. } = &mut self {
            *destructive = true;
        }
        self
    }
}

pub(crate) fn cmd(label: impl Into<SharedString>, icon: &'static str, command: Command) -> Entry {
    item(label, icon, move |b, w, cx| commands::run(b, command, w, cx))
}

fn copy_text(label: impl Into<SharedString>, icon: &'static str, text: String, toast: &'static str) -> Entry {
    item(label, icon, move |b, _, cx| {
        cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
        b.toast(toast, ToastStyle::Success, None, cx);
    })
}

/// Open `url` in a new tab next to the current one without switching to it.
fn open_in_background(b: &mut Browser, url: &str, window: &mut Window, cx: &mut Context<Browser>) -> TabId {
    let id = match b.active_tab() {
        Some(opener) => b.ws.add_child_tab(TabRecord::new(url), opener, None),
        None => b.ws.add_new_tab(TabRecord::new(url)),
    };
    b.realize(id, window, cx);
    b.schedule_save(cx);
    cx.notify();
    id
}

/// Open `url` in a new tab shown beside the current one.
fn open_in_split(b: &mut Browser, url: &str, window: &mut Window, cx: &mut Context<Browser>) {
    let id = open_in_background(b, url, window, cx);
    b.open_split(id, window, cx);
}

fn bookmark(b: &mut Browser, url: &str, title: &str, cx: &mut Context<Browser>) {
    let added = b.bookmarks.toggle(url, title);
    b.toast(if added { "Bookmarked" } else { "Removed bookmark" }, ToastStyle::Success, None, cx);
    b.schedule_save(cx);
}

fn entries(b: &Browser, kind: &MenuKind) -> Vec<Entry> {
    match kind {
        MenuKind::Web { tab, info } => web_entries(b, *tab, info),
        MenuKind::Tab(id) => tab_entries(b, *id),
        MenuKind::Folder(fid) => folder_entries(b, *fid),
        MenuKind::Space(sid) => {
            let sid = *sid;
            let index = b.ws.spaces.iter().position(|s| s.id == sid).unwrap_or(0);
            let count = b.ws.spaces.len();
            vec![
                item("Edit Space…", "palette", move |b, w, cx| b.open_space_editor(sid, None, w, cx)),
                item("Rename Space", "pencil", move |b, w, cx| {
                    b.switch_space(sid, w, cx);
                    b.begin_rename(RenameTarget::Space(sid), w, cx);
                }),
                Entry::Separator,
                item("Move Left", "arrow-left", move |b, _, cx| {
                    b.ws.move_space(sid, index.saturating_sub(1));
                    b.schedule_save(cx);
                })
                .enabled(index > 0),
                item("Move Right", "arrow-right", move |b, _, cx| {
                    b.ws.move_space(sid, index + 2);
                    b.schedule_save(cx);
                })
                .enabled(index + 1 < count),
            ]
            .into_iter()
            .chain(crate::ui::windows::profile_entries(b, sid))
            .chain([
                Entry::Separator,
                cmd("New Space", "layers", Command::NewSpace),
                item("Delete Space", "trash", move |b, w, cx| b.delete_space(sid, w, cx))
                    .enabled(count > 1)
                    .destructive(),
            ])
            .collect()
        }
        MenuKind::Sidebar => vec![
            item("New Tab", "plus", |b, w, cx| b.open_launcher(LauncherMode::NewTab, w, cx))
                .shortcut("secondary-t"),
            item("New Folder", "folder-plus", |b, w, cx| b.new_folder(Vec::new(), w, cx)),
            cmd("New Space", "layers", Command::NewSpace),
            cmd("New Window", "app-window", Command::NewWindow).shortcut("secondary-n"),
            cmd("Private Window", "hat-glasses", Command::NewPrivateWindow).shortcut("secondary-shift-n"),
            Entry::Separator,
            cmd("Reopen Closed Tab", "rotate-ccw", Command::ReopenClosedTab)
                .shortcut("secondary-shift-t")
                .enabled(!b.ws.closed.is_empty()),
            cmd("Sleep Background Tabs", "moon", Command::SleepBackgroundTabs),
            cmd("Peek a Link", "eye", Command::PeekClipboard),
            Entry::Separator,
            item(
                if b.assistant_visible { "Hide Assistant" } else { "Show Assistant" },
                "sparkles",
                |b, w, cx| b.toggle_assistant(w, cx),
            )
            .enabled(b.settings.ai_integration_enabled),
            item("Move Sidebar to Other Side", "arrow-left-right", |b, _, cx| {
                use mori_core::settings::SidebarPosition as P;
                b.settings.sidebar_position = match b.settings.sidebar_position {
                    P::Left => P::Right,
                    P::Right => P::Left,
                };
                b.schedule_save(cx);
            }),
            item("Hide Sidebar", "panel-left-close", |b, _, cx| b.toggle_sidebar(cx))
                .shortcut("secondary-s"),
            item("Edit Theme…", "palette", |b, w, cx| {
                b.show_settings(SettingsCategory::Appearance, w, cx)
            }),
            Entry::Separator,
            cmd("Settings", "settings", Command::Settings).shortcut("secondary-,"),
        ],
        MenuKind::SiteInfo => crate::ui::site_info::site_info_entries(b),
        MenuKind::NewMenu => vec![
            cmd("Create Space", "layers", Command::NewSpace),
            item("Create Folder", "folder-plus", |b, w, cx| b.new_folder(Vec::new(), w, cx)),
            Entry::Separator,
            item("New Split", "columns-2", |b, w, cx| {
                // Pair the current tab with the most recently used other one.
                let current = b.selected();
                let other = b
                    .ws
                    .active()
                    .visible()
                    .into_iter()
                    .filter(|t| Some(*t) != current)
                    .max_by(|x, y| {
                        let at = |id| b.ws.tab(id).map(|t| t.last_accessed).unwrap_or(0.0);
                        at(*x).total_cmp(&at(*y))
                    });
                match other {
                    Some(id) => b.open_split(id, w, cx),
                    None => b.toast("Open another tab to split with", ToastStyle::Info, None, cx),
                }
            })
            .enabled(b.split.is_none()),
            item("New Tab", "plus", |b, w, cx| b.open_launcher(LauncherMode::NewTab, w, cx)),
        ],
    }
}

/// "Outer › Inner" for a nested folder.
fn folder_path(space: &Space, folder: FolderId) -> String {
    space
        .folder_trail(folder)
        .iter()
        .map(|f| f.name.as_str())
        .collect::<Vec<_>>()
        .join(" › ")
}

fn folder_entries(b: &Browser, fid: FolderId) -> Vec<Entry> {
    let Some((space, folder)) = b.ws.find_folder(fid) else { return Vec::new() };
    let depth = b.ws.space(space).and_then(|s| s.folder_depth(fid)).unwrap_or(1);
    let expanded = folder.is_expanded;
    let moves: Vec<Entry> = b
        .ws
        .spaces
        .iter()
        .filter(|s| s.id != space)
        .map(|s| {
            let sid = s.id;
            item(format!("Move to {}", s.name), "arrow-right-left", move |b, w, cx| {
                b.move_folder_to_space(fid, sid, w, cx)
            })
        })
        .collect();
    let mut v = vec![
        item("New Tab in Folder", "plus", move |b, w, cx| {
            let url = b.settings.new_tab_url().to_string();
            let previous = b.selected();
            let id = b.new_tab_with_url(&url, w, cx);
            if let Some((space, _)) = b.ws.find_folder(fid) {
                b.ws.move_tab(id, space, Slot::Folder(fid, usize::MAX));
            }
            b.open_launcher_for_blank(id, previous, w, cx);
        }),
        item("New Subfolder", "folder-plus", move |b, w, cx| {
            if let Some((space, _)) = b.ws.find_folder(fid)
                && let Some(sub) = b.ws.create_folder_at(space, Some(fid), usize::MAX, "New Folder", &[])
            {
                b.begin_rename(RenameTarget::Folder(sub), w, cx);
                b.schedule_save(cx);
            }
        })
        .enabled(depth < MAX_FOLDER_DEPTH),
        item("Rename Folder", "pencil", move |b, w, cx| b.begin_rename(RenameTarget::Folder(fid), w, cx)),
        item(if expanded { "Collapse Folder" } else { "Expand Folder" }, "chevrons-down-up", move |b, _, cx| {
            b.ws.update_folder(fid, |f| f.is_expanded = !f.is_expanded);
            b.schedule_save(cx);
        }),
        Entry::Separator,
        Entry::Header("Icon".into()),
        Entry::FolderIcons {
            folder: fid,
            current: folder.symbol.clone(),
        },
        Entry::Separator,
    ];
    if !moves.is_empty() {
        v.extend(moves);
        v.push(Entry::Separator);
    }
    v.push(item("Ungroup Folder", "folder-open", move |b, _, cx| {
        b.ws.ungroup_folder(fid);
        b.schedule_save(cx);
    }));
    v.push(
        item("Delete Folder", "trash", move |b, _, cx| {
            b.ws.delete_folder(fid);
            b.toast("Folder deleted — its tabs moved below the divider", ToastStyle::Info, None, cx);
            b.schedule_save(cx);
        })
        .destructive(),
    );
    v
}

/// The menu of a tab that is part of a multi-selection: actions on all of
/// them at once.
fn selection_entries(b: &Browser) -> Vec<Entry> {
    let ids = b.ws.in_sidebar_order(&b.multi_select);
    let n = ids.len();
    let all_favorites = ids.iter().all(|t| b.ws.is_favorite(*t));
    let all_pinned = ids.iter().all(|t| b.ws.is_pinned(*t));
    let loose: Vec<TabId> = ids.iter().copied().filter(|t| !b.ws.is_saved(*t)).collect();
    let mut v = vec![Entry::Header(format!("{n} Tabs Selected").into())];
    let group = ids.clone();
    v.push(item(if all_pinned { "Unpin Tabs" } else { "Pin Tabs" }, "pin", move |b, _, cx| {
        let space = b.ws.active_space;
        if all_pinned {
            b.ws.move_tabs(&group, space, Slot::Loose(0));
        } else {
            b.ws.move_tabs(&group, space, Slot::Pinned(usize::MAX));
        }
        b.multi_select.clear();
        b.schedule_save(cx);
    }));
    let group = ids.clone();
    v.push(item(
        if all_favorites { "Remove from Favorites" } else { "Add to Favorites" },
        "star",
        move |b, _, cx| {
            let space = b.ws.active_space;
            let ok = if all_favorites {
                b.ws.move_tabs(&group, space, Slot::Loose(0))
            } else {
                b.ws.move_tabs(&group, space, Slot::Favorite(usize::MAX))
            };
            if !ok {
                b.toast(format!("Favorites hold up to {MAX_FAVORITES} tabs"), ToastStyle::Info, None, cx);
            }
            b.multi_select.clear();
            b.schedule_save(cx);
        },
    ));
    let group = ids.clone();
    v.push(item("Group into New Folder", "folder-plus", move |b, w, cx| b.new_folder(group.clone(), w, cx)));
    for s in b.ws.spaces.iter().filter(|s| s.id != b.ws.active_space) {
        let (sid, group) = (s.id, ids.clone());
        v.push(item(format!("Move to {}", s.name), "arrow-right-left", move |b, w, cx| {
            b.move_tabs_to_space(group.clone(), sid, w, cx);
            b.multi_select.clear();
        }));
    }
    v.push(Entry::Separator);
    let archive = loose.clone();
    v.push(
        item(format!("Archive {} Tabs", archive.len()), "archive", move |b, w, cx| {
            b.multi_select.clear();
            b.archive_tabs(archive.clone(), w, cx);
        })
        .enabled(!loose.is_empty()),
    );
    v.push(
        item(format!("Close {n} Tabs"), "x", move |b, w, cx| {
            let group = std::mem::take(&mut b.multi_select);
            b.close_tabs(group, w, cx);
        })
        .destructive(),
    );
    v.push(item("Clear Selection", "square-dashed", |b, _, cx| {
        b.multi_select.clear();
        cx.notify();
    }));
    v
}

fn tab_entries(b: &Browser, id: TabId) -> Vec<Entry> {
    let Some(record) = b.ws.tab(id) else { return Vec::new() };
    if b.multi_select.len() > 1 && b.multi_select.contains(&id) {
        return selection_entries(b);
    }
    let pinned = b.ws.is_pinned(id);
    let favorite = b.ws.is_favorite(id);
    let saved = pinned || favorite;
    let in_folder = b.ws.folder_of(id);
    let is_web = record.url.starts_with("http");
    let selected = b.selected() == Some(id);
    let live = b.live.contains_key(&id);
    let away = saved && record.left_pinned_url();
    let url = record.url.clone();
    let title = record.display_title().to_string();

    let mut v = vec![
        item("New Tab Below", "plus", move |b, w, cx| {
            let url = b.settings.new_tab_url().to_string();
            let previous = b.selected();
            let new_id = b.ws.add_child_tab(TabRecord::new(url), id, None);
            b.select_tab(new_id, w, cx);
            b.open_launcher_for_blank(new_id, previous, w, cx);
        }),
        Entry::Separator,
        item("Reload Tab", "rotate-cw", move |b, w, cx| match b.live.get(&id) {
            Some(l) => l.view.reload(false),
            None => b.select_tab(id, w, cx),
        }),
    ];
    if live {
        let muted = b.live.get(&id).is_some_and(|l| l.muted);
        v.push(item(if muted { "Unmute Tab" } else { "Mute Tab" }, "volume-x", move |b, _, cx| {
            b.toggle_mute(id, cx)
        }));
    }
    if away {
        v.push(item("Back to Pinned Page", "undo-2", move |b, w, cx| b.reset_pinned_tab(id, w, cx)));
        v.push(item("Replace Pinned URL with Current", "pin", move |b, _, cx| {
            if b.ws.replace_pinned_url(id) {
                b.toast("This page is the pinned one now", ToastStyle::Success, None, cx);
            }
            b.schedule_save(cx);
        }));
    }
    v.push(item(if favorite { "Remove from Favorites" } else { "Add to Favorites" }, "star", move |b, _, cx| {
        b.toggle_favorite(id, cx)
    }));
    if !favorite {
        v.push(item(if pinned { "Unpin Tab" } else { "Pin Tab" }, "pin", move |b, _, cx| {
            b.toggle_pin(id, cx)
        }));
    }
    v.push(item("Duplicate Tab", "copy", move |b, w, cx| b.duplicate_tab(id, w, cx)));
    if is_web {
        let (u, t) = (url.clone(), title.clone());
        let bookmarked = b.bookmarks.contains(&url);
        v.push(item(if bookmarked { "Remove Bookmark" } else { "Bookmark Tab" }, "bookmark", move |b, _, cx| {
            bookmark(b, &u, &t, cx)
        }));
        v.push(copy_text("Copy Link", "link", url.clone(), "Copied link"));
    }
    v.push(item("Rename…", "pencil", move |b, w, cx| b.begin_rename(RenameTarget::Tab(id), w, cx)));
    if record.custom_title.is_some() {
        v.push(item("Reset Name", "rotate-ccw", move |b, _, cx| {
            if let Some(t) = b.ws.tab_mut(id) {
                t.custom_title = None;
            }
            b.schedule_save(cx);
        }));
    }

    v.push(Entry::Separator);
    if !selected && b.ws.space_of(id) == Some(b.ws.active_space) {
        if b.split == Some(id) {
            v.push(item("Close Split View", "columns-2", |b, _, cx| b.close_split(cx)).shortcut("secondary-shift-s"));
        } else {
            v.push(item("Open in Split View", "columns-2", move |b, w, cx| b.open_split(id, w, cx)));
        }
    }
    let space = b.ws.active();
    for (folder, depth) in space.all_folders() {
        if Some(folder.id) == in_folder {
            continue;
        }
        let (fid, sid) = (folder.id, space.id);
        // Nested folders read as a path: "Work › Docs".
        let path = folder_path(space, fid);
        let label = if depth > 1 { path } else { folder.name.clone() };
        v.push(item(format!("Add to {label}"), "folder-input", move |b, _, cx| {
            b.ws.move_tab(id, sid, Slot::Folder(fid, usize::MAX));
            b.schedule_save(cx);
        }));
    }
    v.push(item("Select All Tabs", "list-checks", |b, _, cx| {
        b.multi_select = b.ws.visible_in(b.ws.active_space);
        cx.notify();
    }));
    v.push(item("New Folder with Tab", "folder-plus", move |b, w, cx| b.new_folder(vec![id], w, cx)));
    if in_folder.is_some() {
        v.push(item("Remove from Folder", "folder-output", move |b, _, cx| {
            let sid = b.ws.active_space;
            b.ws.move_tab(id, sid, Slot::Pinned(usize::MAX));
            b.schedule_save(cx);
        }));
    }
    for s in b.ws.spaces.iter().filter(|s| s.id != b.ws.active_space) {
        let sid = s.id;
        v.push(item(format!("Move to {}", s.name), "arrow-right-left", move |b, w, cx| {
            b.move_tab_to_space(id, sid, w, cx);
        }));
    }
    v.extend(crate::ui::windows::move_to_window_entries(b, id));
    if is_web {
        let url = url.clone();
        v.push(item("Always Open in This Space", "route", move |b, _, cx| {
            let space = b.ws.active_space;
            let name = b.ws.active().name.clone();
            match b.routes.add(&url, space) {
                Some(route) => {
                    let p = route.pattern.clone();
                    b.toast(format!("{p} now opens in {name}"), ToastStyle::Success, None, cx);
                }
                None => b.toast("Can't route this address", ToastStyle::Warning, None, cx),
            }
            b.schedule_save(cx);
        }));
    }
    if selected && is_web {
        v.push(Entry::Separator);
        v.push(cmd("Boost This Site…", "zap", Command::BoostSite));
        v.push(cmd("Zap an Element", "eraser", Command::ZapElement));
    }

    v.push(Entry::Separator);
    if live && !selected && !saved {
        v.push(item("Sleep Tab", "moon", move |b, _, cx| b.sleep_tab(id, cx)));
    }
    if saved && (live || away) {
        // Ctrl+W on a pinned tab: back to its page, unloaded, row kept.
        v.push(item("Unload Tab", "minus", move |b, w, cx| b.unload_saved_tab(id, w, cx)).shortcut("secondary-w"));
    }
    v.push(item("Archive Tab", "archive", move |b, w, cx| b.archive_tab(id, w, cx)).enabled(!saved));
    v.push(item("Close Other Tabs", "x", move |b, w, cx| b.close_other_tabs(id, w, cx)));
    if b.ws.active().loose.contains(&id) {
        v.push(item("Close Tabs Below", "x", move |b, w, cx| b.close_tabs_below(id, w, cx)));
    }
    let close = item("Close Tab", "x", move |b, w, cx| b.close_tab(id, true, w, cx));
    v.push(if saved { close } else { close.shortcut("secondary-w") });
    v.push(
        cmd("Reopen Closed Tab", "rotate-ccw", Command::ReopenClosedTab)
            .shortcut("secondary-shift-t")
            .enabled(!b.ws.closed.is_empty()),
    );
    v
}

fn web_entries(b: &Browser, tab: TabId, info: &mori_engine::ContextMenuInfo) -> Vec<Entry> {
    let mut v = Vec::new();
    let view = b.live.get(&tab).map(|l| l.view.clone());
    let engine = b.settings.search_engine.label();
    if let Some(link) = info.link_url.clone() {
        if let Some(text) = info.link_text.clone() {
            v.push(Entry::Header(text.chars().take(48).collect::<String>().into()));
        }
        let l = link.clone();
        v.push(item("Open Link in New Tab", "square-arrow-out-up-right", move |b, w, cx| {
            b.open_url(&l, true, w, cx)
        }));
        let l = link.clone();
        v.push(item("Open Link in Background", "layers", move |b, w, cx| {
            open_in_background(b, &l, w, cx);
        }));
        let l = link.clone();
        v.push(item("Open Link in Split View", "columns-2", move |b, w, cx| open_in_split(b, &l, w, cx)));
        let l = link.clone();
        v.push(item("Open Link in Peek", "eye", move |b, w, cx| b.peek_url(&l, w, cx)).shortcut("alt-click"));
        for s in &b.ws.spaces {
            if s.id == b.ws.active_space {
                continue;
            }
            let (sid, l) = (s.id, link.clone());
            v.push(item(format!("Open Link in {}", s.name), "arrow-right-left", move |b, w, cx| {
                let id = b.new_tab_with_url(&l, w, cx);
                b.move_tab_to_space(id, sid, w, cx);
                b.select_tab(id, w, cx);
            }));
        }
        v.push(Entry::Separator);
        let (l, title) = (link.clone(), info.link_text.clone().unwrap_or_else(|| link.clone()));
        let bookmarked = b.bookmarks.contains(&link);
        v.push(item(if bookmarked { "Remove Bookmark" } else { "Bookmark Link" }, "bookmark", move |b, _, cx| {
            bookmark(b, &l, &title, cx)
        }));
        let (l, vw) = (link.clone(), view.clone());
        v.push(item("Download Linked File", "download", move |_, _, _| {
            if let Some(v) = &vw {
                v.start_download(&l);
            }
        }));
        v.push(copy_text("Copy Link", "link", link.clone(), "Copied link"));
        v.push(Entry::Separator);
    }
    if let Some(src) = info.image_url.clone() {
        let s = src.clone();
        v.push(item("Open Image in New Tab", "image", move |b, w, cx| b.open_url(&s, true, w, cx)));
        let (s, vw) = (src.clone(), view.clone());
        v.push(item("Save Image to Downloads", "download", move |_, _, _| {
            if let Some(v) = &vw {
                v.start_download(&s);
            }
        }));
        v.push(copy_text("Copy Image Address", "link", src.clone(), "Copied image address"));
        let s = src.clone();
        v.push(item("Search Image with Google", "search", move |b, w, cx| {
            let url = format!(
                "https://lens.google.com/uploadbyurl?url={}",
                url::form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>()
            );
            b.open_url(&url, true, w, cx)
        }));
        v.push(Entry::Separator);
    }
    if let Some(src) = info.media_url.clone() {
        let media = |label: &'static str, icon: &'static str, action: MediaAction| {
            let (vw, script) = (view.clone(), page_actions::media_action_script(&src, action));
            item(label, icon, move |_, _, _| {
                if let Some(v) = &vw {
                    v.execute_js(&script);
                }
            })
        };
        v.push(if info.media_paused {
            media("Play", "play", MediaAction::TogglePlay)
        } else {
            media("Pause", "pause", MediaAction::TogglePlay)
        });
        if info.media_has_audio {
            v.push(if info.media_muted {
                media("Unmute", "volume-2", MediaAction::ToggleMute)
            } else {
                media("Mute", "volume-x", MediaAction::ToggleMute)
            });
        }
        v.push(media(if info.media_loop { "Stop Looping" } else { "Loop" }, "repeat", MediaAction::ToggleLoop));
        v.push(media(
            if info.media_controls { "Hide Controls" } else { "Show Controls" },
            "sliders-horizontal",
            MediaAction::ToggleControls,
        ));
        v.push(Entry::Separator);
        let s = src.clone();
        v.push(item("Open Media in New Tab", "square-arrow-out-up-right", move |b, w, cx| {
            b.open_url(&s, true, w, cx)
        }));
        let (s, vw) = (src.clone(), view.clone());
        v.push(item("Save Media to Downloads", "download", move |_, _, _| {
            if let Some(v) = &vw {
                v.start_download(&s);
            }
        }));
        v.push(copy_text("Copy Media Address", "link", src.clone(), "Copied media address"));
        v.push(Entry::Separator);
    }
    if let Some(word) = info.misspelled_word.clone() {
        for s in info.suggestions.iter().take(4) {
            let s = s.clone();
            let vw = view.clone();
            v.push(item(s.clone(), "spell-check", move |_, _, _| {
                if let Some(v) = &vw {
                    v.replace_misspelling(&s);
                }
            }));
        }
        let vw = view.clone();
        v.push(item("Add to Dictionary", "book-plus", move |_, _, _| {
            if let Some(v) = &vw {
                v.add_to_dictionary(&word);
            }
        }));
        v.push(Entry::Separator);
    }
    if info.is_editable {
        let ed = |label: &'static str, icon: &'static str, ok: bool, f: fn(&mori_engine::WebView)| {
            let vw = view.clone();
            item(label, icon, move |_, _, _| {
                if let Some(v) = &vw {
                    f(v)
                }
            })
            .enabled(ok)
        };
        v.push(ed("Undo", "undo-2", info.can_undo, |v| v.undo()).shortcut("secondary-z"));
        v.push(ed("Redo", "redo-2", info.can_redo, |v| v.redo()).shortcut("secondary-shift-z"));
        v.push(Entry::Separator);
        let clip = |label: &'static str, icon: &'static str, ok: bool, f: fn(&mori_engine::WebView)| {
            let vw = view.clone();
            item(label, icon, move |b, _, cx| {
                if let Some(v) = &vw {
                    f(v);
                    b.poll_page_clipboard(v.clone(), cx);
                }
            })
            .enabled(ok)
        };
        v.push(clip("Cut", "scissors", info.can_cut, |v| v.cut()).shortcut("secondary-x"));
        v.push(clip("Copy", "copy", info.can_copy, |v| v.copy()).shortcut("secondary-c"));
        let vw = view.clone();
        // Chromium only knows its own clipboard; the system one may have text.
        v.push(
            item("Paste", "clipboard-paste", move |b, _, cx| {
                if let Some(v) = &vw {
                    b.paste_into(v.clone(), false, cx);
                }
            })
            .shortcut("secondary-v"),
        );
        v.push(ed("Select All", "square-dashed", info.can_select_all, |v| v.select_all()).shortcut("secondary-a"));
        v.push(Entry::Separator);
    } else if let Some(sel) = info.selection.clone() {
        let vw = view.clone();
        v.push(
            item("Copy", "copy", move |b, _, cx| {
                if let Some(v) = &vw {
                    v.copy();
                    b.poll_page_clipboard(v.clone(), cx);
                }
            })
            .shortcut("secondary-c"),
        );
        let label: String = sel.chars().take(24).collect();
        let ellipsis = if sel.chars().count() > 24 { "…" } else { "" };
        v.push(item(format!("Search {engine} for “{label}{ellipsis}”"), "search", move |b, w, cx| {
            let url = b.settings.search_url(&sel);
            b.open_url(&url, true, w, cx)
        }));
        v.push(Entry::Separator);
    }
    let plain = info.link_url.is_none() && info.image_url.is_none() && info.media_url.is_none() && !info.is_editable;
    if plain {
        let can_back = b.live.get(&tab).is_some_and(|l| l.can_go_back);
        let can_fwd = b.live.get(&tab).is_some_and(|l| l.can_go_forward);
        let vw = view.clone();
        v.push(
            item("Back", "arrow-left", move |_, _, _| {
                if let Some(v) = &vw {
                    v.go_back();
                }
            })
            .enabled(can_back),
        );
        let vw = view.clone();
        v.push(
            item("Forward", "arrow-right", move |_, _, _| {
                if let Some(v) = &vw {
                    v.go_forward();
                }
            })
            .enabled(can_fwd),
        );
        let vw = view.clone();
        v.push(
            item("Reload", "rotate-cw", move |_, _, _| {
                if let Some(v) = &vw {
                    v.reload(false);
                }
            })
            .shortcut("secondary-r"),
        );
        v.push(Entry::Separator);
        let page = info.page_url.clone();
        let title = b.ws.tab(tab).map(|t| t.display_title().to_string()).unwrap_or_default();
        if page.starts_with("http") {
            let (p, t) = (page.clone(), title);
            let bookmarked = b.bookmarks.contains(&page);
            v.push(item(if bookmarked { "Remove Bookmark" } else { "Bookmark Page" }, "bookmark", move |b, _, cx| {
                bookmark(b, &p, &t, cx)
            }));
            v.push(copy_text("Copy Page Link", "link", page.clone(), "Copied link"));
        }
        v.push(cmd("Reader View", "book-open-text", Command::ReaderView));
        v.push(cmd("Capture Visible Tab", "camera", Command::CaptureVisibleTab));
        v.push(cmd("Print…", "printer", Command::Print).shortcut("secondary-p"));
        let vw = view.clone();
        v.push(item("View Page Source", "file-code", move |b, w, cx| {
            if let Some(v) = &vw {
                let url = format!("view-source:{}", v.url());
                b.open_url(&url, true, w, cx);
            }
        }));
        v.push(Entry::Separator);
    }
    let (x, y) = (info.x, info.y);
    v.push(item("Inspect", "code", move |b, w, cx| b.inspect_element(tab, x, y, w, cx)));
    v
}

thread_local! {
    /// Last painted bounds of menu triggers, per window (see [`anchor_probe`]).
    static ANCHORS: RefCell<HashMap<(WindowId, SharedString), Bounds<Pixels>>> = RefCell::default();
}

/// An invisible overlay that records where its (relatively positioned)
/// parent was painted, so a menu opened from it can be placed beside it
/// instead of on top of it. Look the bounds up with [`anchor_bounds`].
pub fn anchor_probe(key: impl Into<SharedString>) -> impl IntoElement {
    let key = key.into();
    canvas(
        move |bounds, window, _| {
            let id = window.window_handle().window_id();
            ANCHORS.with(|a| a.borrow_mut().insert((id, key), bounds));
        },
        |_, _, _, _| {},
    )
    .absolute()
    .inset_0()
}

pub fn anchor_bounds(key: impl Into<SharedString>, window: &Window) -> Option<Bounds<Pixels>> {
    let id = window.window_handle().window_id();
    ANCHORS.with(|a| a.borrow().get(&(id, key.into())).copied())
}

/// Anchor key of a space's chip in the sidebar's space switcher.
pub fn space_chip_anchor(space: SpaceId) -> SharedString {
    format!("space-chip-{space}").into()
}

/// Top-left corner for a menu of `size` in a `viewport`, as (x, y).
///
/// With an `anchor` (a button) the menu opens below it, or above when it
/// doesn't fit, left-aligned with it or right-aligned when that fits better.
/// Otherwise it opens to the bottom-right of `cursor`, flipping up or left
/// when it would leave the screen, so it doesn't sit on the point clicked.
/// It is clamped into an 8px margin as a last resort.
fn place_menu(viewport: (f32, f32), size: (f32, f32), cursor: (f32, f32), anchor: Option<[f32; 4]>) -> (f32, f32) {
    const MARGIN: f32 = 8.;
    let (vw, vh) = viewport;
    let (w, h) = size;
    let fits_x = |x: f32| x >= MARGIN && x + w <= vw - MARGIN;
    let fits_y = |y: f32| y >= MARGIN && y + h <= vh - MARGIN;
    let (x, y) = match anchor {
        Some([left, top, right, bottom]) => {
            let below = bottom + MENU_GAP;
            let above = top - MENU_GAP - h;
            let y = if fits_y(below) { below } else if fits_y(above) { above } else { below };
            let x = if fits_x(left) { left } else { right - w };
            (x, y)
        }
        None => {
            let (cx, cy) = cursor;
            let x = if fits_x(cx) || !fits_x(cx - w) { cx } else { cx - w };
            let y = if fits_y(cy) || !fits_y(cy - h) { cy } else { cy - h };
            (x, y)
        }
    };
    (
        x.min(vw - MARGIN - w).max(MARGIN),
        y.min(vh - MARGIN - h).max(MARGIN),
    )
}

impl Browser {
    /// Close the open menu and hand focus back to the page.
    pub fn close_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.menu.take().is_some() {
            self.focus_selected_page(window, cx);
            cx.notify();
        }
    }

    pub fn render_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.menu.clone()?;
        let t = cx.tokens().clone();
        let list = entries(self, &menu.kind);
        if list.is_empty() {
            self.menu = None;
            return None;
        }
        // Take focus from the page so Esc reaches the menu.
        if !self.focus_handle.is_focused(window) {
            cx.defer_in(window, |this, window, cx| {
                if this.menu.is_some() {
                    window.focus(&this.focus_handle, cx);
                }
            });
        }
        let height: f32 = list.iter().map(Entry::height).sum::<f32>() + MENU_PADDING * 2. + 2.;
        let viewport = window.viewport_size();
        // Opened from a known trigger (the click landed on it): sit beside it.
        let anchor = match &menu.kind {
            MenuKind::NewMenu => anchor_bounds(NEW_MENU_ANCHOR, window),
            MenuKind::SiteInfo => anchor_bounds(crate::ui::site_info::OMNIBOX_ANCHOR, window),
            MenuKind::Space(sid) => anchor_bounds(space_chip_anchor(*sid), window),
            // Right-clicks elsewhere in the sidebar miss the button and keep
            // the menu at the cursor.
            MenuKind::Sidebar => anchor_bounds(APP_MENU_ANCHOR, window),
            _ => None,
        }
        .filter(|b| b.contains(&menu.position))
        .map(|b| [f32::from(b.left()), f32::from(b.top()), f32::from(b.right()), f32::from(b.bottom())]);
        let (x, y) = place_menu(
            (f32::from(viewport.width), f32::from(viewport.height)),
            (MENU_WIDTH, height),
            (f32::from(menu.position.x), f32::from(menu.position.y)),
            anchor,
        );

        let hover_bg = t.foreground.opacity(if t.dark { 0.10 } else { 0.07 });
        let separator = t.popover_separator();
        let rows: Vec<AnyElement> = list
            .into_iter()
            .enumerate()
            .map(|(i, e)| match e {
                Entry::Separator => div()
                    .my(px(4.))
                    .mx(px(6.))
                    .h(px(1.))
                    .bg(separator)
                    .into_any_element(),
                Entry::Header(text) => div()
                    .h(px(HEADER_HEIGHT))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .text_size(metrics::TEXT_LABEL)
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(t.muted_foreground)
                    .truncate()
                    .child(text)
                    .into_any_element(),
                Entry::Note(text) => div()
                    .min_h(px(HEADER_HEIGHT))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .text_size(metrics::TEXT_LABEL)
                    .text_color(t.muted_foreground)
                    .child(text)
                    .into_any_element(),
                Entry::Custom { render, .. } => render(window, cx),
                Entry::Switch {
                    label,
                    icon: ic,
                    on,
                    handler,
                } => h_flex()
                    .id(("menu-switch", i))
                    .h(px(ITEM_HEIGHT))
                    .flex_none()
                    .px(px(10.))
                    .gap(px(10.))
                    .rounded(px(5.))
                    .cursor_pointer()
                    .hover(|s| s.bg(hover_bg))
                    .child(icon(ic, px(15.), t.foreground.opacity(0.6)))
                    .child(div().flex_1().min_w_0().truncate().child(label))
                    .child(
                        div()
                            .w(px(28.))
                            .h(px(16.))
                            .flex_none()
                            .p(px(2.))
                            .rounded_full()
                            .bg(if on { t.primary } else { t.foreground.opacity(0.18) })
                            .child(div().size(px(12.)).rounded_full().bg(white()).when(on, |s| s.ml(px(12.)))),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| handler(this, window, cx)))
                    .into_any_element(),
                Entry::FolderIcons { folder, current } => self.render_icon_grid(folder, &current, cx),
                Entry::Item {
                    label,
                    icon: ic,
                    shortcut,
                    enabled,
                    destructive,
                    handler,
                } => {
                    let fg = if !enabled {
                        t.muted_foreground.opacity(0.6)
                    } else if destructive {
                        t.destructive
                    } else {
                        t.foreground
                    };
                    let icon_color = if destructive { fg } else { fg.opacity(0.6) };
                    h_flex()
                        .id(("menu-item", i))
                        .h(px(ITEM_HEIGHT))
                        .flex_none()
                        .px(px(10.))
                        .gap(px(10.))
                        .rounded(px(5.))
                        .text_color(fg)
                        .when(enabled, |s| s.cursor_pointer().hover(|s| s.bg(hover_bg)))
                        .when_some(ic, |s, ic| s.child(icon(ic, px(15.), icon_color)))
                        .child(div().flex_1().min_w_0().truncate().child(label))
                        .when_some(shortcut, |s, k| {
                            s.child(
                                div()
                                    .flex_none()
                                    .text_size(metrics::TEXT_LABEL)
                                    .text_color(t.muted_foreground.opacity(0.85))
                                    .child(crate::actions::shortcut_label(k)),
                            )
                        })
                        .when(enabled, |s| {
                            s.on_click(cx.listener(move |this, _, window, cx| {
                                this.close_menu(window, cx);
                                handler(this, window, cx);
                            }))
                        })
                        .into_any_element()
                }
            })
            .collect();

        let border = t.popover_border();
        Some(
            div()
                .absolute()
                .inset_0()
                .occlude()
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| this.close_menu(window, cx)))
                .on_mouse_down(MouseButton::Right, cx.listener(|this, _, window, cx| this.close_menu(window, cx)))
                .child(
                    div()
                        .id("context-menu")
                        .absolute()
                        .left(px(x))
                        .top(px(y))
                        .w(px(MENU_WIDTH))
                        .max_h(viewport.height - px(16.))
                        .overflow_y_scroll()
                        .p(px(MENU_PADDING))
                        // Zen's menu surface: #1f1f1f dark, #f4f4f4 light.
                        .bg(if t.dark { Hsla::from(rgb(0x1f1f1f)) } else { Hsla::from(rgb(0xf4f4f4)) })
                        .rounded(px(8.))
                        .border_1()
                        .border_color(border)
                        .shadow(vec![BoxShadow {
                            color: black().opacity(t.shadow_alpha(0.12, 0.30)),
                            offset: point(px(0.), px(8.)),
                            blur_radius: px(24.),
                            spread_radius: px(0.),
                            inset: false,
                        }])
                        .text_color(t.foreground)
                        .text_size(MENU_TEXT)
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                        .child(v_flex().children(rows)),
                )
                .into_any_element(),
        )
    }

    fn render_icon_grid(&self, folder: FolderId, current: &str, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.tokens().clone();
        let cells = crate::ui::sidebar::FOLDER_SYMBOLS.iter().enumerate().map(|(i, symbol)| {
            let symbol: &'static str = symbol;
            let active = symbol == current;
            div()
                .id(("folder-icon", i))
                .size(px(ICON_CELL - 2.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .cursor_pointer()
                .when(active, |s| s.bg(t.primary.opacity(0.22)).border_1().border_color(t.primary.opacity(0.8)))
                .when(!active, |s| s.hover(|s| s.bg(t.foreground.opacity(0.08))))
                .child(icon(symbol, px(15.), if active { t.foreground } else { t.foreground.opacity(0.75) }))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.ws.update_folder(folder, |f| f.symbol = symbol.to_string());
                    this.close_menu(window, cx);
                    this.schedule_save(cx);
                }))
        });
        div()
            .px(px(6.))
            .py(px(4.))
            .grid()
            .grid_cols(ICON_COLUMNS as u16)
            .gap(px(2.))
            .children(cells)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::place_menu;

    const VIEW: (f32, f32) = (1440., 900.);
    const MENU: (f32, f32) = (256., 300.);

    #[test]
    fn opens_bottom_right_of_the_cursor() {
        assert_eq!(place_menu(VIEW, MENU, (100., 100.), None), (100., 100.));
    }

    #[test]
    fn flips_instead_of_covering_the_cursor() {
        // Near the bottom-right corner: up and to the left of the point.
        assert_eq!(place_menu(VIEW, MENU, (1400., 850.), None), (1144., 550.));
    }

    #[test]
    fn clamps_when_neither_side_fits() {
        let tall = (256., 880.);
        assert_eq!(place_menu(VIEW, tall, (100., 400.), None), (100., 12.));
        // Wider than the window: pinned to the left margin.
        assert_eq!(place_menu((200., 900.), MENU, (100., 100.), None), (8., 100.));
    }

    #[test]
    fn button_menus_open_beside_the_button() {
        // A "+" button at the bottom of the sidebar: above it, left-aligned.
        let button = [226., 858., 256., 888.];
        assert_eq!(place_menu(VIEW, (256., 160.), (240., 870.), Some(button)), (226., 694.));
        // A button near the top: below it.
        let top = [10., 10., 40., 40.];
        assert_eq!(place_menu(VIEW, (256., 160.), (20., 20.), Some(top)), (10., 44.));
        // Too close to the right edge: right-aligned with the button.
        let right = [1400., 10., 1430., 40.];
        assert_eq!(place_menu(VIEW, (256., 160.), (1410., 20.), Some(right)), (1174., 44.));
    }
}
