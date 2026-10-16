//! The launcher: a command palette for opening URLs, searching, jumping to
//! tabs and history, and running commands (Ctrl+T / Ctrl+L).

use std::{collections::HashMap, sync::Arc};

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    component::input::{Escape, IndentInline, Input, InputEvent, InputState, MoveDown, MoveUp},
    *,
};
use mori_core::{
    launcher::{self, Candidate, ItemKind},
    omnibox,
    util::host_of,
    workspace::TabId,
};

use crate::{
    browser::Browser,
    commands::Command,
    theme::{ActiveTokens, metrics},
    ui::components::{favicon, h_flex, icon, shortcut_caps, v_flex},
};

const MAX_RESULTS: usize = 8;
const ROW_HEIGHT: Pixels = px(48.);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LauncherMode {
    /// Ctrl+T: results open in a new tab.
    NewTab,
    /// Ctrl+L / clicking the address pill: results replace the current page.
    Address,
}

pub enum LauncherEvent {
    Dismiss,
    OpenInput(String),
    OpenUrl(String),
    SwitchToTab(TabId),
    Command(Command),
    AskAssistant(String),
}

#[derive(Clone)]
enum Payload {
    Tab(TabId),
    Url(String),
    Command(Command),
}

struct CommandEntry {
    icon: &'static str,
    shortcut: Option<&'static str>,
}

/// Everything the launcher can show, snapshotted when it opens.
pub struct Candidates {
    items: Vec<Candidate>,
    payloads: HashMap<String, Payload>,
    commands: HashMap<String, CommandEntry>,
    favicons: HashMap<String, Arc<RenderImage>>,
    ai_enabled: bool,
}

pub fn candidates(b: &mut Browser, mode: LauncherMode) -> Candidates {
    let mut items = Vec::new();
    let mut payloads = HashMap::new();
    let mut commands = HashMap::new();
    let mut hosts = Vec::new();
    let current = (mode == LauncherMode::Address).then(|| b.selected()).flatten();

    for t in &b.ws.tabs {
        if Some(t.id) == current || Browser::is_internal(&t.url) {
            continue;
        }
        let key = format!("tab:{}", t.id);
        items.push(Candidate {
            key: key.clone(),
            kind: ItemKind::Tab,
            title: t.display_title().to_string(),
            url: Some(t.url.clone()),
            keywords: String::new(),
            recency: t.last_accessed,
        });
        payloads.insert(key, Payload::Tab(t.id));
        hosts.extend(host_of(&t.url));
    }
    for h in b.history.recent(2000) {
        let key = format!("history:{}", h.url);
        items.push(Candidate {
            key: key.clone(),
            kind: ItemKind::History,
            title: if h.title.is_empty() { h.url.clone() } else { h.title.clone() },
            url: Some(h.url.clone()),
            keywords: String::new(),
            recency: h.last_visited,
        });
        payloads.insert(key, Payload::Url(h.url.clone()));
        hosts.extend(host_of(&h.url));
    }
    for bm in b.bookmarks.list() {
        let key = format!("bookmark:{}", bm.url);
        items.push(Candidate {
            key: key.clone(),
            kind: ItemKind::Bookmark,
            title: bm.title.clone(),
            url: Some(bm.url.clone()),
            keywords: String::new(),
            recency: bm.created_at,
        });
        payloads.insert(key, Payload::Url(bm.url.clone()));
        hosts.extend(host_of(&bm.url));
    }
    for (i, info) in crate::commands::available(b).into_iter().enumerate() {
        let key = format!("command:{i}");
        items.push(Candidate {
            key: key.clone(),
            kind: ItemKind::Command,
            title: info.title.clone(),
            url: None,
            keywords: info.keywords.to_string(),
            recency: 0.0,
        });
        payloads.insert(key.clone(), Payload::Command(info.command));
        commands.insert(
            key,
            CommandEntry {
                icon: info.icon,
                shortcut: info.shortcut,
            },
        );
    }
    hosts.sort();
    hosts.dedup();
    let favicons = hosts
        .into_iter()
        .filter_map(|h| b.favicons.get(&h).map(|img| (h, img)))
        .collect();
    Candidates {
        items,
        payloads,
        commands,
        favicons,
        ai_enabled: b.settings.ai_integration_enabled,
    }
}

#[derive(Clone)]
enum Row {
    Open(String),
    Search(String),
    Item(Candidate),
    Ask(String),
}

pub struct Launcher {
    mode: LauncherMode,
    input: Entity<InputState>,
    data: Candidates,
    rows: Vec<Row>,
    highlighted: usize,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<LauncherEvent> for Launcher {}

impl Launcher {
    pub fn new(
        mode: LauncherMode,
        prefill: String,
        data: Candidates,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("Search or Enter URL…");
            if !prefill.is_empty() {
                s.set_value(prefill, window, cx);
            }
            s
        });
        let sub = cx.subscribe_in(&input, window, |this, _, ev: &InputEvent, window, cx| match ev {
            InputEvent::Change => {
                this.refresh(cx);
            }
            InputEvent::PressEnter { .. } => this.activate(this.highlighted, window, cx),
            InputEvent::Blur => {}
            InputEvent::Focus => {}
        });
        let mut this = Launcher {
            mode,
            input,
            data,
            rows: Vec::new(),
            highlighted: 0,
            _subscriptions: vec![sub],
        };
        this.refresh(cx);
        this
    }

    pub fn mode(&self) -> LauncherMode {
        self.mode
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |s, cx| {
            s.focus(window, cx);
            s.select_all(window, cx);
        });
    }

    fn query(&self, cx: &App) -> String {
        self.input.read(cx).value().to_string()
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let query = self.query(cx);
        let q = query.trim();
        let mut rows = Vec::new();
        if !q.is_empty() {
            if omnibox::looks_like_address(q) {
                rows.push(Row::Open(q.to_string()));
            } else {
                rows.push(Row::Search(q.to_string()));
            }
        }
        // In address mode the prefilled URL shouldn't drown in suggestions.
        let ranked = launcher::rank(q, self.data.items.clone(), MAX_RESULTS);
        let room = MAX_RESULTS.saturating_sub(rows.len() + usize::from(!q.is_empty() && self.data.ai_enabled));
        rows.extend(ranked.into_iter().take(room).map(Row::Item));
        if !q.is_empty() && self.data.ai_enabled {
            rows.push(Row::Ask(q.to_string()));
        }
        self.rows = rows;
        self.highlighted = 0;
        cx.notify();
    }

    fn move_highlight(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.rows.is_empty() {
            return;
        }
        let n = self.rows.len() as isize;
        self.highlighted = (self.highlighted as isize + delta).rem_euclid(n) as usize;
        cx.notify();
    }

    fn activate(&mut self, index: usize, _window: &mut Window, cx: &mut Context<Self>) {
        let query = self.query(cx);
        let Some(row) = self.rows.get(index).cloned() else {
            cx.emit(LauncherEvent::OpenInput(query));
            return;
        };
        let event = match row {
            Row::Open(text) | Row::Search(text) => LauncherEvent::OpenInput(text),
            Row::Ask(text) => LauncherEvent::AskAssistant(text),
            Row::Item(c) => match self.data.payloads.get(&c.key).cloned() {
                Some(Payload::Tab(id)) => LauncherEvent::SwitchToTab(id),
                Some(Payload::Url(url)) => LauncherEvent::OpenUrl(url),
                Some(Payload::Command(cmd)) => LauncherEvent::Command(cmd),
                None => LauncherEvent::OpenInput(query),
            },
        };
        cx.emit(event);
    }

    fn render_row(&self, i: usize, row: &Row, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.tokens().clone();
        let highlighted = i == self.highlighted;
        let (leading, title, subtitle, trailing, shortcut): (AnyElement, String, Option<String>, &str, Option<&str>) =
            match row {
                Row::Open(text) => (
                    icon("globe", px(18.), t.muted_foreground).into_any_element(),
                    format!("Open {text}"),
                    None,
                    "Open",
                    None,
                ),
                Row::Search(text) => (
                    icon("search", px(18.), t.muted_foreground).into_any_element(),
                    format!("Search “{text}”"),
                    None,
                    "Search",
                    None,
                ),
                Row::Ask(text) => (
                    icon("sparkles", px(18.), t.primary).into_any_element(),
                    format!("Ask Assistant: {text}"),
                    None,
                    "Ask",
                    None,
                ),
                Row::Item(c) => {
                    let url = c.url.clone().unwrap_or_default();
                    match c.kind {
                        ItemKind::Command => {
                            let entry = self.data.commands.get(&c.key);
                            (
                                icon(entry.map(|e| e.icon).unwrap_or("command"), px(18.), t.muted_foreground)
                                    .into_any_element(),
                                c.title.clone(),
                                None,
                                "Run",
                                entry.and_then(|e| e.shortcut),
                            )
                        }
                        kind => {
                            let fav = host_of(&url).and_then(|h| self.data.favicons.get(&h).cloned());
                            (
                                favicon(fav, px(18.), false, cx),
                                c.title.clone(),
                                Some(omnibox::display_url(&url)),
                                if kind == ItemKind::Tab { "Switch to Tab" } else { "Open" },
                                None,
                            )
                        }
                    }
                }
            };
        let is_tab = matches!(row, Row::Item(c) if c.kind == ItemKind::Tab);
        let show_trailing = highlighted || is_tab || shortcut.is_some();
        let highlight_bg = if t.dark { white().opacity(0.07) } else { black().opacity(0.05) };
        h_flex()
            .id(("launcher-row", i))
            .h(ROW_HEIGHT)
            .flex_none()
            .px(px(10.))
            .gap_3()
            .rounded(px(8.))
            .when(highlighted, |s| s.bg(highlight_bg))
            .cursor_pointer()
            .on_mouse_move(cx.listener(move |this, _, _, cx| {
                if this.highlighted != i {
                    this.highlighted = i;
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |this, _, window, cx| this.activate(i, window, cx)))
            .child(div().flex_none().size(px(18.)).child(leading))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(metrics::TEXT_BASE)
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(t.foreground)
                            .truncate()
                            .child(title),
                    )
                    .when_some(subtitle.filter(|s| !s.is_empty()), |s, sub| {
                        s.child(
                            div()
                                .text_size(metrics::TEXT_LABEL)
                                .text_color(t.muted_foreground)
                                .truncate()
                                .child(sub),
                        )
                    }),
            )
            .when(show_trailing, |s| {
                s.child(
                    h_flex()
                        .flex_none()
                        .gap_2()
                        .child(
                            div()
                                .text_size(metrics::TEXT_LABEL)
                                .text_color(t.muted_foreground)
                                .child(trailing.to_string()),
                        )
                        .when_some(shortcut, |s, keys| s.child(shortcut_caps(keys, cx))),
                )
            })
    }
}

impl Render for Launcher {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens().clone();
        let viewport = window.viewport_size();
        let top = viewport.height * 0.24;
        let rows: Vec<_> = self
            .rows
            .clone()
            .iter()
            .enumerate()
            .map(|(i, r)| self.render_row(i, r, cx).into_any_element())
            .collect();
        let empty = rows.is_empty();
        let border = if t.dark { white().opacity(0.10) } else { black().opacity(0.06) };

        div()
            .absolute()
            .inset_0()
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.emit(LauncherEvent::Dismiss)),
            )
            .child(
                v_flex()
                    .id("launcher")
                    .absolute()
                    .top(top)
                    .left_0()
                    .right_0()
                    .mx_auto()
                    .w(px(620.).min(viewport.width - px(48.)))
                    .bg(t.popover)
                    .rounded(metrics::RADIUS_POPOVER)
                    .border_1()
                    .border_color(border)
                    .shadow(t.modal_shadow())
                    .text_color(t.foreground)
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .capture_action(cx.listener(|this, _: &MoveUp, _, cx| {
                        this.move_highlight(-1, cx);
                        cx.stop_propagation();
                    }))
                    .capture_action(cx.listener(|this, _: &MoveDown, _, cx| {
                        this.move_highlight(1, cx);
                        cx.stop_propagation();
                    }))
                    .capture_action(cx.listener(|this, _: &IndentInline, _, cx| {
                        let q = this.query(cx);
                        if !q.trim().is_empty() && this.data.ai_enabled {
                            cx.emit(LauncherEvent::AskAssistant(q));
                        }
                        cx.stop_propagation();
                    }))
                    .capture_action(cx.listener(|_, _: &Escape, _, cx| {
                        cx.emit(LauncherEvent::Dismiss);
                        cx.stop_propagation();
                    }))
                    .child(
                        h_flex()
                            .h(px(52.))
                            .px_4()
                            .gap_3()
                            .child(icon("search", px(16.), t.foreground.opacity(0.65)))
                            .child(
                                div().flex_1().text_size(metrics::TEXT_TITLE).child(
                                    Input::new(&self.input)
                                        .appearance(false)
                                        .bordered(false)
                                        .focus_bordered(false),
                                ),
                            )
                            .when(self.data.ai_enabled, |s| {
                                s.child(
                                    h_flex()
                                        .gap_1()
                                        .text_size(metrics::TEXT_SMALL)
                                        .text_color(t.muted_foreground)
                                        .child(
                                            div()
                                                .px_1()
                                                .rounded(metrics::RADIUS_MD)
                                                .bg(t.foreground.opacity(0.07))
                                                .child("TAB"),
                                        )
                                        .child("for Assistant"),
                                )
                            }),
                    )
                    .child(div().h(px(1.)).bg(t.border.opacity(0.5)))
                    .child(
                        v_flex()
                            .id("launcher-results")
                            .p(px(6.))
                            .gap(px(1.))
                            .max_h(ROW_HEIGHT * 6.5)
                            .overflow_y_scroll()
                            .when(empty, |s| {
                                s.child(
                                    div()
                                        .py_3()
                                        .px_3()
                                        .text_size(metrics::TEXT_LABEL)
                                        .text_color(t.muted_foreground)
                                        .child("Type to search, open a URL, or run a command"),
                                )
                            })
                            .children(rows),
                    ),
            )
    }
}
