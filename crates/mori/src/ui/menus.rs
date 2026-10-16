//! Context menus: page, tab, folder, space, sidebar background, and "+".

use std::rc::Rc;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use mori_core::workspace::TabId;

use crate::{
    browser::{Browser, MenuKind, NEW_TAB_URL, RenameTarget, ToastStyle},
    commands::{self, Command},
    theme::{ActiveTokens, metrics},
    ui::{
        components::{h_flex, icon, popover_surface, shortcut_caps, v_flex},
        launcher::LauncherMode,
    },
};

type Handler = Rc<dyn Fn(&mut Browser, &mut Window, &mut Context<Browser>)>;

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

fn entries(b: &Browser, kind: &MenuKind) -> Vec<Entry> {
    match kind {
        MenuKind::Web { tab, info } => web_entries(b, *tab, info),
        MenuKind::Tab(id) => tab_entries(b, *id),
        MenuKind::Folder(fid) => {
            let fid = *fid;
            vec![
                item("Rename Folder", "pencil", move |b, w, cx| {
                    b.begin_rename(RenameTarget::Folder(fid), w, cx)
                }),
                item("New Tab in Folder", "plus", move |b, w, cx| {
                    let id = b.new_tab_with_url(NEW_TAB_URL, w, cx);
                    if let Some((space, f)) = b.ws.find_folder(fid) {
                        let end = f.tab_ids.len();
                        b.ws.move_tab(id, space, mori_core::workspace::Slot::Folder(fid, end));
                    }
                    b.open_launcher(LauncherMode::Address, w, cx);
                }),
                Entry::Separator,
                item("Delete Folder", "trash-2", move |b, _, cx| {
                    b.ws.delete_folder(fid);
                    b.schedule_save(cx);
                })
                .destructive(),
            ]
        }
        MenuKind::Space(sid) => {
            let sid = *sid;
            let mut v = vec![item("Rename Space", "pencil", move |b, w, cx| {
                b.switch_space(sid, w, cx);
                b.begin_rename(RenameTarget::Space(sid), w, cx);
            })];
            v.push(Entry::Header("Icon".into()));
            for chunk in crate::ui::sidebar::SPACE_SYMBOLS.chunks(1).take(12) {
                let symbol = chunk[0];
                v.push(item(symbol.replace('-', " "), symbol, move |b, _, cx| {
                    if let Some(s) = b.ws.space_mut(sid) {
                        s.symbol = symbol.to_string();
                    }
                    b.schedule_save(cx);
                }));
            }
            v.push(Entry::Separator);
            v.push(cmd("New Space", "layers", Command::NewSpace));
            v.push(
                item("Delete Space", "trash-2", move |b, w, cx| b.delete_space(sid, w, cx))
                    .enabled(b.ws.spaces.len() > 1)
                    .destructive(),
            );
            v
        }
        MenuKind::Sidebar => vec![
            item("New Tab", "plus", |b, w, cx| b.open_launcher(LauncherMode::NewTab, w, cx))
                .shortcut("secondary-t"),
            item("New Folder", "folder-plus", |b, w, cx| b.new_folder(Vec::new(), w, cx)),
            Entry::Separator,
            cmd("Sleep Background Tabs", "moon", Command::SleepBackgroundTabs),
            cmd("Peek a Link", "eye", Command::PeekClipboard),
            cmd("Capture Visible Tab", "camera", Command::CaptureVisibleTab),
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
            Entry::Separator,
            cmd("Settings", "settings", Command::Settings),
        ],
        MenuKind::NewMenu => vec![
            cmd("New Space", "layers", Command::NewSpace),
            item("New Folder", "folder-plus", |b, w, cx| b.new_folder(Vec::new(), w, cx)),
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
    let mut v = vec![item("Rename…", "pencil", move |b, w, cx| {
        b.begin_rename(RenameTarget::Tab(id), w, cx)
    })];
    if record.custom_title.is_some() {
        v.push(item("Reset Name", "rotate-ccw", move |b, _, cx| {
            if let Some(t) = b.ws.tab_mut(id) {
                t.custom_title = None;
            }
            b.schedule_save(cx);
        }));
    }
    v.push(Entry::Separator);
    v.push(item(if pinned { "Unpin Tab" } else { "Pin Tab" }, "pin", move |b, _, cx| {
        b.toggle_pin(id, cx)
    }));
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
        v.push(item("New Folder with Tab", "folder-plus", move |b, w, cx| {
            b.new_folder(vec![id], w, cx)
        }));
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
        let url = record.url.clone();
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
    if !selected && b.ws.space_of(id) == Some(b.ws.active_space) {
        if b.split == Some(id) {
            v.push(item("Close Split View", "columns-2", |b, _, cx| b.close_split(cx)).shortcut("secondary-shift-s"));
        } else {
            v.push(item("Open in Split View", "columns-2", move |b, w, cx| b.open_split(id, w, cx)));
        }
    }
    v.push(Entry::Separator);
    v.push(item("Duplicate Tab", "copy", move |b, w, cx| b.duplicate_tab(id, w, cx)));
    let url = record.url.clone();
    v.push(item("Copy URL", "link", move |b, _, cx| {
        cx.write_to_clipboard(ClipboardItem::new_string(url.clone()));
        b.toast("Copied URL", ToastStyle::Success, None, cx);
    }));
    if live && !selected {
        v.push(item("Sleep Tab", "moon", move |b, _, cx| b.sleep_tab(id, cx)));
    }
    v.push(item("Archive Tab", "archive", move |b, w, cx| b.archive_tab(id, w, cx)).enabled(!pinned));
    if selected && is_web {
        v.push(Entry::Separator);
        v.push(cmd("Boost This Site…", "zap", Command::BoostSite));
        v.push(cmd("Zap an Element", "eraser", Command::ZapElement));
    }
    if live {
        let muted = b.live.get(&id).is_some_and(|l| l.muted);
        v.push(item(if muted { "Unmute Tab" } else { "Mute Tab" }, "volume-x", move |b, _, cx| {
            b.toggle_mute(id, cx)
        }));
    }
    v.push(Entry::Separator);
    v.push(item("Reload", "rotate-cw", move |b, w, cx| match b.live.get(&id) {
        Some(l) => l.view.reload(false),
        None => {
            b.select_tab(id, w, cx);
        }
    }));
    v.push(item("Close Other Tabs", "x", move |b, w, cx| b.close_other_tabs(id, w, cx)));
    v.push(item("Close Tabs Below", "x", move |b, w, cx| b.close_tabs_below(id, w, cx)));
    v.push(item("Close Tab", "x", move |b, w, cx| b.close_tab(id, true, w, cx)).shortcut("secondary-w"));
    v
}

fn web_entries(b: &Browser, tab: TabId, info: &mori_engine::ContextMenuInfo) -> Vec<Entry> {
    let mut v = Vec::new();
    let view = b.live.get(&tab).map(|l| l.view.clone());
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
            let current = b.selected();
            b.open_url(&l, true, w, cx);
            if let Some(c) = current {
                b.select_tab(c, w, cx);
            }
        }));
        let l = link.clone();
        v.push(item("Open Link in Peek", "eye", move |b, w, cx| b.peek_url(&l, w, cx)));
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
        let l = link.clone();
        v.push(item("Copy Link Address", "link", move |_, _, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(l.clone()))
        }));
        v.push(Entry::Separator);
    }
    if let Some(src) = info.image_url.clone() {
        let s = src.clone();
        v.push(item("Open Image in New Tab", "image", move |b, w, cx| b.open_url(&s, true, w, cx)));
        let (s, vw) = (src.clone(), view.clone());
        v.push(item("Save Image", "download", move |_, _, _| {
            if let Some(v) = &vw {
                v.start_download(&s);
            }
        }));
        let s = src.clone();
        v.push(item("Copy Image Address", "link", move |_, _, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(s.clone()))
        }));
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
        v.push(ed("Undo", "undo-2", info.can_undo, |v| v.undo()));
        v.push(ed("Redo", "redo-2", info.can_redo, |v| v.redo()));
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
        v.push(clip("Cut", "scissors", info.can_cut, |v| v.cut()));
        v.push(clip("Copy", "copy", info.can_copy, |v| v.copy()));
        let vw = view.clone();
        // Chromium only knows its own clipboard; the system one may have text.
        v.push(item("Paste", "clipboard-paste", move |b, _, cx| {
            if let Some(v) = &vw {
                b.paste_into(v.clone(), false, cx);
            }
        }));
        v.push(ed("Select All", "text-select", info.can_select_all, |v| v.select_all()));
        v.push(Entry::Separator);
    } else if let Some(sel) = info.selection.clone() {
        let vw = view.clone();
        v.push(item("Copy", "copy", move |b, _, cx| {
            if let Some(v) = &vw {
                v.copy();
                b.poll_page_clipboard(v.clone(), cx);
            }
        }));
        let label: String = sel.chars().take(24).collect();
        v.push(item(format!("Search for “{label}”"), "search", move |b, w, cx| {
            let url = b.settings.search_url(&sel);
            b.open_url(&url, true, w, cx)
        }));
        v.push(Entry::Separator);
    }
    if info.link_url.is_none() && info.image_url.is_none() && !info.is_editable {
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
        v.push(item("Reload", "rotate-cw", move |_, _, _| {
            if let Some(v) = &vw {
                v.reload(false);
            }
        }));
        v.push(Entry::Separator);
        v.push(cmd("Reader View", "book-open-text", Command::ReaderView));
        v.push(cmd("Capture Visible Tab", "camera", Command::CaptureVisibleTab));
        v.push(cmd("Print…", "printer", Command::Print));
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

impl Browser {
    pub fn render_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.menu.clone()?;
        let t = cx.tokens().clone();
        let list = entries(self, &menu.kind);
        if list.is_empty() {
            self.menu = None;
            return None;
        }
        const WIDTH: f32 = 240.;
        let item_h = 30.;
        let height: f32 = list
            .iter()
            .map(|e| match e {
                Entry::Separator => 9.,
                Entry::Header(_) => 26.,
                Entry::Item { .. } => item_h,
            })
            .sum::<f32>()
            + 12.;
        let viewport = window.viewport_size();
        let x = f32::from(menu.position.x).min(f32::from(viewport.width) - WIDTH - 8.).max(8.);
        let y = f32::from(menu.position.y)
            .min(f32::from(viewport.height) - height - 8.)
            .max(8.);

        let rows: Vec<AnyElement> = list
            .into_iter()
            .enumerate()
            .map(|(i, e)| match e {
                Entry::Separator => div()
                    .my(px(4.))
                    .mx_2()
                    .h(px(1.))
                    .bg(t.border.opacity(0.6))
                    .into_any_element(),
                Entry::Header(text) => div()
                    .h(px(26.))
                    .px_2()
                    .flex()
                    .items_center()
                    .text_size(metrics::TEXT_SMALL)
                    .text_color(t.muted_foreground)
                    .truncate()
                    .child(text)
                    .into_any_element(),
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
                    h_flex()
                        .id(("menu-item", i))
                        .h(px(item_h))
                        .px_2()
                        .gap_2()
                        .rounded(metrics::RADIUS_LG)
                        .text_color(fg)
                        .when(enabled, |s| {
                            s.cursor_pointer().hover(|s| {
                                s.bg(t.primary).text_color(t.primary_foreground)
                            })
                        })
                        .when_some(ic, |s, ic| s.child(icon(ic, px(14.), fg.opacity(0.85))))
                        .child(div().flex_1().truncate().child(label))
                        .when_some(shortcut, |s, k| s.child(shortcut_caps(k, cx)))
                        .when(enabled, |s| {
                            s.on_click(cx.listener(move |this, _, window, cx| {
                                this.menu = None;
                                handler(this, window, cx);
                                cx.notify();
                            }))
                        })
                        .into_any_element()
                }
            })
            .collect();

        Some(
            div()
                .absolute()
                .inset_0()
                .occlude()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.menu = None;
                        cx.notify();
                    }),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|this, _, _, cx| {
                        this.menu = None;
                        cx.notify();
                    }),
                )
                .child(
                    popover_surface(cx)
                        .id("context-menu")
                        .absolute()
                        .left(px(x))
                        .top(px(y))
                        .w(px(WIDTH))
                        .max_h(viewport.height - px(16.))
                        .overflow_y_scroll()
                        .p(px(6.))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(v_flex().children(rows)),
                )
                .into_any_element(),
        )
    }
}
