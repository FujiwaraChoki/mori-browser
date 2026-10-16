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
    workspace::{SpaceId, TabId, TabRecord},
};

use crate::{
    browser::{Browser, MenuKind, NEW_TAB_URL, RenameTarget, ToastStyle},
    commands::{self, Command},
    theme::{ActiveTokens, metrics},
    ui::{
        components::{h_flex, icon, v_flex},
        launcher::LauncherMode,
        settings::SettingsCategory,
    },
};

type Handler = Rc<dyn Fn(&mut Browser, &mut Window, &mut Context<Browser>)>;

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
    Separator,
    /// The space glyph picker: every symbol, the current one marked.
    Icons { space: SpaceId, current: String },
}

impl Entry {
    fn height(&self) -> f32 {
        match self {
            Entry::Separator => SEPARATOR_HEIGHT,
            Entry::Header(_) => HEADER_HEIGHT,
            Entry::Item { .. } => ITEM_HEIGHT,
            Entry::Icons { .. } => {
                let rows = crate::ui::sidebar::SPACE_SYMBOLS.len().div_ceil(ICON_COLUMNS);
                rows as f32 * ICON_CELL + 8.
            }
        }
    }
}

fn item(
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

trait EntryExt {
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

fn cmd(label: impl Into<SharedString>, icon: &'static str, command: Command) -> Entry {
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
    let after = b.selected().filter(|s| !b.ws.is_pinned(*s));
    let id = b.ws.add_tab_after(TabRecord::new(url), after);
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
        MenuKind::Folder(fid) => {
            let fid = *fid;
            vec![
                item("New Tab in Folder", "plus", move |b, w, cx| {
                    let id = b.new_tab_with_url(NEW_TAB_URL, w, cx);
                    if let Some((space, f)) = b.ws.find_folder(fid) {
                        let end = f.tab_ids.len();
                        b.ws.move_tab(id, space, mori_core::workspace::Slot::Folder(fid, end));
                    }
                    b.open_launcher(LauncherMode::Address, w, cx);
                }),
                item("Rename Folder", "pencil", move |b, w, cx| {
                    b.begin_rename(RenameTarget::Folder(fid), w, cx)
                }),
                Entry::Separator,
                item("Delete Folder", "trash", move |b, _, cx| {
                    b.ws.delete_folder(fid);
                    b.schedule_save(cx);
                })
                .destructive(),
            ]
        }
        MenuKind::Space(sid) => {
            let sid = *sid;
            let current = b.ws.space(sid).map(|s| s.symbol.clone()).unwrap_or_default();
            vec![
                item("Rename Space", "pencil", move |b, w, cx| {
                    b.switch_space(sid, w, cx);
                    b.begin_rename(RenameTarget::Space(sid), w, cx);
                }),
                item("Edit Theme…", "palette", move |b, w, cx| {
                    b.switch_space(sid, w, cx);
                    b.show_settings(SettingsCategory::Appearance, w, cx);
                }),
                Entry::Separator,
                Entry::Header("Icon".into()),
                Entry::Icons { space: sid, current },
                Entry::Separator,
                cmd("New Space", "layers", Command::NewSpace),
                item("Delete Space", "trash", move |b, w, cx| b.delete_space(sid, w, cx))
                    .enabled(b.ws.spaces.len() > 1)
                    .destructive(),
            ]
        }
        MenuKind::Sidebar => vec![
            item("New Tab", "plus", |b, w, cx| b.open_launcher(LauncherMode::NewTab, w, cx))
                .shortcut("secondary-t"),
            item("New Folder", "folder-plus", |b, w, cx| b.new_folder(Vec::new(), w, cx)),
            cmd("New Space", "layers", Command::NewSpace),
            Entry::Separator,
            cmd("Reopen Closed Tab", "rotate-ccw", Command::ReopenClosedTab)
                .shortcut("secondary-shift-t")
                .enabled(!b.closed_stack.is_empty()),
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
        MenuKind::NewMenu => vec![
            cmd("New Space", "layers", Command::NewSpace),
            item("New Folder", "folder-plus", |b, w, cx| b.new_folder(Vec::new(), w, cx)),
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
            Entry::Separator,
            item("New Tab", "plus", |b, w, cx| b.open_launcher(LauncherMode::NewTab, w, cx))
                .shortcut("secondary-t"),
        ],
    }
}

fn tab_entries(b: &Browser, id: TabId) -> Vec<Entry> {
    let Some(record) = b.ws.tab(id) else { return Vec::new() };
    let pinned = b.ws.is_pinned(id);
    let in_folder = b.ws.folder_of(id);
    let is_web = record.url.starts_with("http");
    let selected = b.selected() == Some(id);
    let live = b.live.contains_key(&id);
    let url = record.url.clone();
    let title = record.display_title().to_string();

    let mut v = vec![
        item("New Tab Below", "plus", move |b, w, cx| {
            let new_id = b.ws.add_tab_after(TabRecord::new(NEW_TAB_URL), Some(id));
            b.select_tab(new_id, w, cx);
            b.open_launcher(LauncherMode::Address, w, cx);
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
    v.push(item(if pinned { "Unpin Tab" } else { "Pin Tab" }, "pin", move |b, _, cx| {
        b.toggle_pin(id, cx)
    }));
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
    for folder in &space.folders {
        if Some(folder.id) == in_folder {
            continue;
        }
        let (fid, sid) = (folder.id, space.id);
        v.push(item(format!("Add to {}", folder.name), "folder-input", move |b, _, cx| {
            let end = b.ws.space(sid).and_then(|s| s.folder(fid)).map(|f| f.tab_ids.len()).unwrap_or(0);
            b.ws.move_tab(id, sid, mori_core::workspace::Slot::Folder(fid, end));
            b.schedule_save(cx);
        }));
    }
    if b.multi_select.len() >= 2 && b.multi_select.contains(&id) {
        let n = b.multi_select.len();
        v.push(item(format!("New Folder with {n} Selected Tabs"), "folder-plus", |b, w, cx| {
            let tabs = b.multi_select.clone();
            b.new_folder(tabs, w, cx);
        }));
    } else {
        v.push(item("New Folder with Tab", "folder-plus", move |b, w, cx| b.new_folder(vec![id], w, cx)));
    }
    if in_folder.is_some() {
        v.push(item("Remove from Folder", "folder-output", move |b, _, cx| {
            let sid = b.ws.active_space;
            b.ws.move_tab(id, sid, mori_core::workspace::Slot::Loose(0));
            b.schedule_save(cx);
        }));
    }
    for s in b.ws.spaces.iter().filter(|s| s.id != b.ws.active_space) {
        let sid = s.id;
        v.push(item(format!("Move to {}", s.name), "arrow-right-left", move |b, w, cx| {
            b.move_tab_to_space(id, sid, w, cx);
        }));
    }
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
    if live && !selected {
        v.push(item("Sleep Tab", "moon", move |b, _, cx| b.sleep_tab(id, cx)));
    }
    v.push(item("Archive Tab", "archive", move |b, w, cx| b.archive_tab(id, w, cx)).enabled(!pinned));
    v.push(item("Close Other Tabs", "x", move |b, w, cx| b.close_other_tabs(id, w, cx)));
    v.push(item("Close Tabs Below", "x", move |b, w, cx| b.close_tabs_below(id, w, cx)));
    v.push(item("Close Tab", "x", move |b, w, cx| b.close_tab(id, true, w, cx)).shortcut("secondary-w"));
    v.push(
        cmd("Reopen Closed Tab", "rotate-ccw", Command::ReopenClosedTab)
            .shortcut("secondary-shift-t")
            .enabled(!b.closed_stack.is_empty()),
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
    let vw = view;
    v.push(item("Inspect", "code", move |_, _, _| {
        if let Some(v) = &vw {
            v.show_devtools(Some((x, y)));
        }
    }));
    v
}

thread_local! {
    /// Last painted bounds of menu triggers, per window (see [`anchor_probe`]).
    static ANCHORS: RefCell<HashMap<(WindowId, &'static str), Bounds<Pixels>>> = RefCell::default();
}

/// An invisible overlay that records where its (relatively positioned)
/// parent was painted, so a menu opened from it can be placed beside it
/// instead of on top of it. Look the bounds up with [`anchor_bounds`].
pub fn anchor_probe(key: &'static str) -> impl IntoElement {
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

pub fn anchor_bounds(key: &'static str, window: &Window) -> Option<Bounds<Pixels>> {
    let id = window.window_handle().window_id();
    ANCHORS.with(|a| a.borrow().get(&(id, key)).copied())
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
        let anchor = match menu.kind {
            MenuKind::NewMenu => anchor_bounds(NEW_MENU_ANCHOR, window),
            _ => None,
        }
        .map(|b| [f32::from(b.left()), f32::from(b.top()), f32::from(b.right()), f32::from(b.bottom())]);
        let (x, y) = place_menu(
            (f32::from(viewport.width), f32::from(viewport.height)),
            (MENU_WIDTH, height),
            (f32::from(menu.position.x), f32::from(menu.position.y)),
            anchor,
        );

        let hover_bg = t.foreground.opacity(if t.dark { 0.10 } else { 0.07 });
        let separator = t.foreground.opacity(if t.dark { 0.10 } else { 0.09 });
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
                Entry::Icons { space, current } => self.render_icon_grid(space, &current, cx),
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

        let border = if t.dark { white().opacity(0.12) } else { black().opacity(0.10) };
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
                        .bg(t.popover)
                        .rounded(px(8.))
                        .border_1()
                        .border_color(border)
                        .shadow(vec![BoxShadow {
                            color: black().opacity(t.shadow_alpha(0.18, 0.45)),
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

    fn render_icon_grid(&self, space: SpaceId, current: &str, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.tokens().clone();
        let cells = crate::ui::sidebar::SPACE_SYMBOLS.iter().enumerate().map(|(i, symbol)| {
            let symbol: &'static str = symbol;
            let active = symbol == current;
            div()
                .id(("space-icon", i))
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
                    if let Some(s) = this.ws.space_mut(space) {
                        s.symbol = symbol.to_string();
                    }
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
