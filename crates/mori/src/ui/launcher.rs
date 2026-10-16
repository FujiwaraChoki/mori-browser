//! The launcher: a floating command bar for opening URLs, searching, jumping
//! to tabs and history, and running commands (Ctrl+T / Ctrl+L).
//!
//! Laid out like Zen's floating URL bar: a wide panel near the top of the
//! window, a large input, and one-line "Title — source" rows. Search
//! completions are fetched from the engine's suggest endpoint off the UI
//! thread, debounced, and merged in under the first row.

use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
    time::Duration,
};

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    component::input::{Escape, IndentInline, Input, InputEvent, InputState, MoveDown, MoveUp},
    *,
};
use mori_core::{
    launcher::{self, Candidate, ItemKind},
    omnibox,
    settings::SearchEngine,
    util::host_of,
    workspace::TabId,
};

use crate::{
    browser::Browser,
    commands::Command,
    theme::{ActiveTokens, metrics},
    ui::components::{favicon, h_flex, icon, shortcut_caps, v_flex},
};

/// Most rows shown at once, including the first row, completions and Ask.
const MAX_ROWS: usize = 10;
/// Rows reserved for tabs/history/bookmarks/commands when matches exist.
const MIN_ITEMS: usize = 4;
const ROW_HEIGHT: Pixels = px(44.);
const INPUT_ROW_HEIGHT: Pixels = px(62.);
const INPUT_TEXT: Pixels = px(17.);
const ROW_TEXT: Pixels = px(14.);
/// Quiet time after a keystroke before completions are requested.
const SUGGEST_DEBOUNCE: Duration = Duration::from_millis(120);
const SUGGEST_TIMEOUT: Duration = Duration::from_secs(3);

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
    engine: SearchEngine,
    suggestions_enabled: bool,
}

/// Snapshot the launcher's candidates. The current tab and history or
/// bookmark rows for the current page are left out in both modes: the bar is
/// for going somewhere else.
pub fn candidates(b: &mut Browser) -> Candidates {
    let mut items = Vec::new();
    let mut payloads = HashMap::new();
    let mut commands = HashMap::new();
    let mut hosts = Vec::new();
    let current = b.selected();
    let current_url = current.and_then(|id| b.ws.tab(id)).map(|t| t.url.clone()).unwrap_or_default();
    let is_current_page = |url: &str| omnibox::same_page(url, &current_url);

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
            visits: 0,
        });
        payloads.insert(key, Payload::Tab(t.id));
        hosts.extend(host_of(&t.url));
    }
    for h in b.history.recent(2000) {
        if is_current_page(&h.url) {
            continue;
        }
        let key = format!("history:{}", h.url);
        items.push(Candidate {
            key: key.clone(),
            kind: ItemKind::History,
            title: h.display_title().to_string(),
            url: Some(h.url.clone()),
            keywords: String::new(),
            recency: h.last_visited,
            visits: h.visit_count,
        });
        payloads.insert(key, Payload::Url(h.url.clone()));
        hosts.extend(host_of(&h.url));
    }
    for bm in b.bookmarks.list() {
        if is_current_page(&bm.url) {
            continue;
        }
        let key = format!("bookmark:{}", bm.url);
        items.push(Candidate {
            key: key.clone(),
            kind: ItemKind::Bookmark,
            title: bm.title.clone(),
            url: Some(bm.url.clone()),
            keywords: String::new(),
            recency: bm.created_at,
            visits: 0,
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
            visits: 0,
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
        engine: b.settings.search_engine,
        suggestions_enabled: b.settings.search_suggestions,
    }
}

#[derive(Clone)]
enum Row {
    Open(String),
    Search(String),
    Suggestion(String),
    Item(Candidate),
    Ask(String),
}

impl Row {
    /// Identity used to keep the highlight on the same row when completions
    /// arrive and shift the list.
    fn key(&self) -> String {
        match self {
            Row::Open(t) => format!("open:{t}"),
            Row::Search(t) => format!("search:{t}"),
            Row::Suggestion(t) => format!("suggest:{t}"),
            Row::Item(c) => c.key.clone(),
            Row::Ask(t) => format!("ask:{t}"),
        }
    }
}

pub struct Launcher {
    mode: LauncherMode,
    input: Entity<InputState>,
    /// The text the bar opened with (the current URL in address mode).
    prefill: String,
    data: Candidates,
    rows: Vec<Row>,
    /// `None` until the user types or arrows: Enter then opens the input as
    /// typed (a blank new tab for an empty Ctrl+T, the page itself for an
    /// untouched Ctrl+L).
    highlighted: Option<usize>,
    /// Completions and the query they were fetched for.
    suggestions: Vec<String>,
    suggestions_for: String,
    suggest_seq: u64,
    _suggest_task: Option<Task<()>>,
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
            let mut s = InputState::new(window, cx).placeholder("Search or enter address…");
            if !prefill.is_empty() {
                s.set_value(prefill.clone(), window, cx);
            }
            s
        });
        let sub = cx.subscribe_in(&input, window, |this, _, ev: &InputEvent, window, cx| match ev {
            InputEvent::Change => this.refresh(cx),
            InputEvent::PressEnter { .. } => this.activate(this.highlighted, window, cx),
            InputEvent::Blur | InputEvent::Focus => {}
        });
        let mut this = Launcher {
            mode,
            input,
            prefill,
            data,
            rows: Vec::new(),
            highlighted: None,
            suggestions: Vec::new(),
            suggestions_for: String::new(),
            suggest_seq: 0,
            _suggest_task: None,
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

    pub fn query(&self, cx: &App) -> String {
        self.input.read(cx).value().to_string()
    }

    /// True while the input is empty or still shows the URL it opened with.
    fn is_idle(&self, query: &str) -> bool {
        let q = query.trim();
        q.is_empty() || (self.mode == LauncherMode::Address && q == self.prefill.trim())
    }

    /// The input changed: rebuild rows and (re)request completions.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let query = self.query(cx);
        self.highlighted = (!self.is_idle(&query)).then_some(0);
        self.rebuild_rows(&query, None, cx);
        self.request_suggestions(&query, cx);
    }

    /// Recompute rows for `query`. With `keep` set, the highlight follows
    /// that row if it is still listed.
    fn rebuild_rows(&mut self, query: &str, keep: Option<String>, cx: &mut Context<Self>) {
        let q = query.trim();
        let idle = self.is_idle(query);
        let mut rows = Vec::new();
        let address = omnibox::looks_like_address(q);
        if !idle {
            rows.push(if address { Row::Open(q.to_string()) } else { Row::Search(q.to_string()) });
        }
        let ask = !idle && !address && self.data.ai_enabled;
        let lower = q.to_lowercase();
        let suggestions: Vec<Row> = if idle || address {
            Vec::new()
        } else {
            // Completions for an earlier prefix stay until fresh ones land,
            // as long as they still extend what's typed.
            self.suggestions
                .iter()
                .filter(|s| s.to_lowercase().starts_with(&lower) && s.to_lowercase() != lower)
                .take(omnibox::MAX_SUGGESTIONS)
                .cloned()
                .map(Row::Suggestion)
                .collect()
        };
        let rank_query = if idle { "" } else { q };
        let ranked = launcher::rank(rank_query, self.data.items.clone(), MAX_ROWS);
        // Completions come first, but matches always keep MIN_ITEMS rows.
        let free = MAX_ROWS.saturating_sub(rows.len() + usize::from(ask));
        let item_room = free.saturating_sub(suggestions.len()).max(MIN_ITEMS).min(ranked.len());
        rows.extend(suggestions.into_iter().take(free.saturating_sub(item_room)));
        rows.extend(ranked.into_iter().take(item_room).map(Row::Item));
        if ask {
            rows.push(Row::Ask(q.to_string()));
        }
        if let Some(key) = keep {
            self.highlighted = rows.iter().position(|r| r.key() == key).or(self.highlighted.map(|_| 0));
        }
        if self.highlighted.is_some_and(|h| h >= rows.len()) {
            self.highlighted = (!rows.is_empty()).then_some(0);
        }
        self.rows = rows;
        cx.notify();
    }

    fn request_suggestions(&mut self, query: &str, cx: &mut Context<Self>) {
        let q = query.trim().to_string();
        let wanted = self.data.suggestions_enabled && !self.is_idle(query) && !omnibox::looks_like_address(&q);
        let url = wanted.then(|| omnibox::suggest_url(self.data.engine, &q)).flatten();
        self.suggest_seq += 1;
        let Some(url) = url else {
            self._suggest_task = None;
            self.suggestions.clear();
            self.suggestions_for.clear();
            return;
        };
        if q == self.suggestions_for {
            return;
        }
        let seq = self.suggest_seq;
        // Replacing the task drops (cancels) any request still in flight.
        self._suggest_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SUGGEST_DEBOUNCE).await;
            let body = cx.background_executor().spawn(async move { fetch_text(&url) }).await;
            let list = match body {
                Ok(body) => omnibox::parse_suggestions(&body, &q, omnibox::MAX_SUGGESTIONS * 2),
                Err(e) => {
                    log::debug!("search suggestions: {e:#}");
                    return;
                }
            };
            this.update(cx, |this, cx| {
                if this.suggest_seq != seq {
                    return;
                }
                this.suggestions = list;
                this.suggestions_for = q;
                let keep = this.highlighted.and_then(|h| this.rows.get(h)).map(Row::key);
                let query = this.query(cx);
                this.rebuild_rows(&query, keep, cx);
            })
            .ok();
        }));
    }

    fn move_highlight(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.rows.is_empty() {
            return;
        }
        let n = self.rows.len() as isize;
        let next = match self.highlighted {
            Some(h) => (h as isize + delta).rem_euclid(n),
            None if delta > 0 => 0,
            None => n - 1,
        };
        self.highlighted = Some(next as usize);
        cx.notify();
    }

    fn activate(&mut self, index: Option<usize>, _window: &mut Window, cx: &mut Context<Self>) {
        let query = self.query(cx);
        let Some(row) = index.and_then(|i| self.rows.get(i)).cloned() else {
            cx.emit(LauncherEvent::OpenInput(query));
            return;
        };
        let event = match row {
            Row::Open(text) | Row::Search(text) | Row::Suggestion(text) => LauncherEvent::OpenInput(text),
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

    fn search_label(&self) -> String {
        match self.data.engine {
            SearchEngine::Custom => "Search".to_string(),
            engine => format!("Search with {}", engine.label()),
        }
    }

    fn render_row(&self, i: usize, row: &Row, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.tokens().clone();
        let highlighted = self.highlighted == Some(i);
        let muted = t.muted_foreground;
        let glyph = |name: &str, color: Hsla| icon(name, px(16.), color).into_any_element();
        let (leading, title, source, shortcut): (AnyElement, String, Option<String>, Option<&str>) = match row {
            Row::Open(text) => (glyph("globe", muted), text.clone(), Some("Visit".into()), None),
            Row::Search(text) => (glyph("search", muted), text.clone(), Some(self.search_label()), None),
            Row::Suggestion(text) => (glyph("search", muted), text.clone(), None, None),
            Row::Ask(text) => (glyph("sparkles", t.primary), text.clone(), Some("Ask Assistant".into()), None),
            Row::Item(c) if c.kind == ItemKind::Command => {
                let entry = self.data.commands.get(&c.key);
                (
                    glyph(entry.map(|e| e.icon).unwrap_or("command"), muted),
                    c.title.clone(),
                    None,
                    entry.and_then(|e| e.shortcut),
                )
            }
            Row::Item(c) => {
                let url = c.url.clone().unwrap_or_default();
                let fav = host_of(&url).and_then(|h| self.data.favicons.get(&h).cloned());
                let source = omnibox::row_host(&url);
                (
                    favicon(fav, px(16.), false, cx),
                    c.title.clone(),
                    (!source.is_empty() && source != c.title).then_some(source),
                    None,
                )
            }
        };
        let is_tab = matches!(row, Row::Item(c) if c.kind == ItemKind::Tab);
        let highlight_bg = t.primary.opacity(if t.dark { 0.32 } else { 0.16 });
        h_flex()
            .id(("launcher-row", i))
            .h(ROW_HEIGHT)
            .flex_none()
            .px(px(12.))
            .gap(px(12.))
            .rounded(px(8.))
            .text_size(ROW_TEXT)
            .when(highlighted, |s| s.bg(highlight_bg))
            .cursor_pointer()
            .on_mouse_move(cx.listener(move |this, _, _, cx| {
                if this.highlighted != Some(i) {
                    this.highlighted = Some(i);
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |this, _, window, cx| this.activate(Some(i), window, cx)))
            .child(div().flex_none().size(px(16.)).flex().items_center().justify_center().child(leading))
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(6.))
                    .child(
                        div()
                            .min_w_0()
                            .flex_shrink(1.)
                            .truncate()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(t.foreground)
                            .child(title),
                    )
                    .when_some(source, |s, source| {
                        s.child(div().flex_none().text_color(muted.opacity(0.7)).child("—")).child(
                            div()
                                .flex_none()
                                .max_w(px(320.))
                                .truncate()
                                .text_color(if highlighted { t.foreground.opacity(0.75) } else { muted })
                                .child(source),
                        )
                    }),
            )
            .when(is_tab, |s| {
                s.child(
                    h_flex()
                        .flex_none()
                        .gap(px(8.))
                        .text_size(metrics::TEXT_LABEL)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(if highlighted { t.foreground.opacity(0.8) } else { muted })
                        .child("Switch to Tab")
                        .child(
                            div()
                                .size(px(20.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(metrics::RADIUS_MD)
                                .bg(t.foreground.opacity(0.08))
                                .child(icon("arrow-right", px(12.), muted)),
                        ),
                )
            })
            .when_some(shortcut, |s, keys| s.child(shortcut_caps(keys, cx)))
    }
}

/// GET `url` and return the body as text. Blocking; run it off the UI thread.
fn fetch_text(url: &str) -> anyhow::Result<String> {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    let agent = AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .timeout_global(Some(SUGGEST_TIMEOUT))
            .user_agent(format!("Mori/{}", env!("CARGO_PKG_VERSION")))
            .build()
            .into()
    });
    Ok(agent.get(url).call()?.body_mut().read_to_string()?)
}

impl Render for Launcher {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens().clone();
        let viewport = window.viewport_size();
        let width = (viewport.width * 0.63).max(px(560.)).min(px(910.)).min(viewport.width - px(32.));
        let top = (viewport.height * 0.3).min(px(280.)).max(px(16.));
        let results_max = (viewport.height - top - INPUT_ROW_HEIGHT - px(24.)).max(ROW_HEIGHT * 2.);
        let rows: Vec<_> = self
            .rows
            .clone()
            .iter()
            .enumerate()
            .map(|(i, r)| self.render_row(i, r, cx).into_any_element())
            .collect();
        let has_rows = !rows.is_empty();
        let border = t.popover_border();
        let typed = !self.is_idle(&self.query(cx));
        let shadow = vec![
            BoxShadow {
                color: black().opacity(t.shadow_alpha(0.22, 0.5)),
                offset: point(px(0.), px(24.)),
                blur_radius: px(110.),
                spread_radius: px(0.),
                inset: false,
            },
            BoxShadow {
                color: black().opacity(t.shadow_alpha(0.10, 0.3)),
                offset: point(px(0.), px(4.)),
                blur_radius: px(14.),
                spread_radius: px(0.),
                inset: false,
            },
        ];

        let panel = v_flex()
            .id("launcher")
            .absolute()
            .top(top)
            .left((viewport.width - width) / 2.)
            .w(width)
            .bg(t.popover)
            .rounded(metrics::RADIUS)
            .border_1()
            .border_color(border)
            .shadow(shadow)
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
                    .h(INPUT_ROW_HEIGHT)
                    .flex_none()
                    .px(px(20.))
                    .gap(px(14.))
                    .child(icon("search", px(16.), t.foreground.opacity(0.55)))
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(&self.input)
                                .appearance(false)
                                .bordered(false)
                                .focus_bordered(false)
                                .px_0()
                                .text_size(INPUT_TEXT)
                                .line_height(px(24.)),
                        ),
                    )
                    .when(self.data.ai_enabled && typed, |s| {
                        s.child(
                            h_flex()
                                .flex_none()
                                .gap_1()
                                .text_size(metrics::TEXT_SMALL)
                                .text_color(t.muted_foreground)
                                .child(
                                    div()
                                        .px_1()
                                        .rounded(metrics::RADIUS_MD)
                                        .bg(t.foreground.opacity(0.08))
                                        .child("Tab"),
                                )
                                .child("Ask Assistant"),
                        )
                    }),
            )
            .when(has_rows, |s| {
                s.child(div().mx(px(10.)).h(px(1.)).bg(t.foreground.opacity(if t.dark { 0.08 } else { 0.07 })))
                    .child(
                        v_flex()
                            .id("launcher-results")
                            .p(px(8.))
                            .gap(px(2.))
                            .max_h(results_max)
                            .overflow_y_scroll()
                            .children(rows),
                    )
            })
            .with_animation(
                "launcher-open",
                Animation::new(Duration::from_millis(160)).with_easing(ease_out_quint()),
                move |el, delta| el.opacity(delta).top(top - px(8.) * (1. - delta)),
            );

        div()
            .absolute()
            .inset_0()
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.emit(LauncherEvent::Dismiss)),
            )
            .child(panel)
    }
}
