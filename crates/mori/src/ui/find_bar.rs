//! Find-in-page bar, pinned to the top-right of the web card.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    component::input::{Escape, Input, InputEvent, InputState},
    *,
};

use crate::{
    theme::{ActiveTokens, metrics},
    ui::components::{h_flex, icon, icon_button, popover_surface},
};

pub enum FindBarEvent {
    Search {
        text: String,
        forward: bool,
        next: bool,
        match_case: bool,
    },
    Close,
}

pub struct FindBar {
    input: Entity<InputState>,
    count: i32,
    active: i32,
    /// Whether the user stepped through the current query's matches; until
    /// then a settled result without an active match is stepped once (see
    /// [`FindBar::set_results`]).
    primed: bool,
    match_case: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<FindBarEvent> for FindBar {}

impl FindBar {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Find in page"));
        let sub = cx.subscribe_in(&input, window, |this, _, ev: &InputEvent, _, cx| match ev {
            InputEvent::Change => this.search(false, cx),
            InputEvent::PressEnter { shift, .. } => this.step(!*shift, cx),
            _ => {}
        });
        FindBar {
            input,
            count: 0,
            active: 0,
            primed: false,
            match_case: false,
            _subscriptions: vec![sub],
        }
    }

    /// Start a new search for the current text. With `restart`, the old
    /// session is dropped first so the search starts over instead of from
    /// the previous match (the query is the same, only options changed).
    fn search(&mut self, restart: bool, cx: &mut Context<Self>) {
        self.count = 0;
        self.active = 0;
        self.primed = false;
        let text = self.text(cx);
        if restart && !text.is_empty() {
            cx.emit(FindBarEvent::Search {
                text: String::new(),
                forward: true,
                next: false,
                match_case: self.match_case,
            });
        }
        // A changed query sent as "find next" starts a new session that
        // selects the first match at or after the current one, like Chrome's
        // find-as-you-type. A plain new session would only highlight, and
        // stepping from the previous query's match afterwards skips the
        // match the selection sits on ("appl" -> "apple" lands on 2/6).
        cx.emit(FindBarEvent::Search {
            text,
            forward: true,
            next: true,
            match_case: self.match_case,
        });
        cx.notify();
    }

    /// Keep `subscription` (the owner's handler for this bar's events) alive
    /// exactly as long as the bar, so reopening never stacks handlers.
    pub fn hold(&mut self, subscription: Subscription) {
        self._subscriptions.push(subscription);
    }

    fn text(&self, cx: &App) -> String {
        self.input.read(cx).value().to_string()
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |s, cx| {
            s.focus(window, cx);
            s.select_all(window, cx);
        });
    }

    pub fn step(&mut self, forward: bool, cx: &mut Context<Self>) {
        let text = self.text(cx);
        if !text.is_empty() {
            self.primed = true;
            cx.emit(FindBarEvent::Search {
                text,
                forward,
                next: true,
                match_case: self.match_case,
            });
        }
    }

    /// A find result from the page. Intermediate updates report ordinal 0,
    /// so the last real ordinal is kept. Should a search settle with matches
    /// but none active, step to the first one so it is selected (and
    /// scrolled to) from the first keystroke, like Chrome.
    pub fn set_results(&mut self, count: i32, active: i32, final_update: bool, cx: &mut Context<Self>) {
        self.count = count;
        if active > 0 || count == 0 {
            self.active = active;
        }
        if final_update && count > 0 && self.active == 0 && !self.primed {
            self.step(true, cx);
        }
        cx.notify();
    }

    /// The selected page changed: search it for the current text (or just
    /// clear the count), so the bar never shows the old page's results or
    /// "No results" for a page that has matches.
    pub fn rerun(&mut self, cx: &mut Context<Self>) {
        if self.text(cx).is_empty() {
            self.count = 0;
            self.active = 0;
            self.primed = false;
            cx.notify();
        } else {
            self.search(true, cx);
        }
    }
}

impl Render for FindBar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens().clone();
        let has_text = !self.text(cx).is_empty();
        let label = if !has_text {
            String::new()
        } else if self.count == 0 {
            "No results".to_string()
        } else {
            format!("{}/{}", self.active, self.count)
        };
        popover_surface(cx)
            .absolute()
            .top(px(10.))
            .right(px(14.))
            .rounded(metrics::RADIUS)
            .occlude()
            .capture_action(cx.listener(|_, _: &Escape, _, cx| {
                cx.emit(FindBarEvent::Close);
                cx.stop_propagation();
            }))
            .child(
                h_flex()
                    .h(px(40.))
                    .pl(px(12.))
                    .pr(px(6.))
                    .gap_2()
                    .child(icon("search", px(14.), t.muted_foreground))
                    .child(
                        div().w(px(200.)).child(
                            Input::new(&self.input)
                                .appearance(false)
                                .bordered(false)
                                .focus_bordered(false)
                                .px_0()
                                .text_size(metrics::TEXT_BASE),
                        ),
                    )
                    .child(
                        div()
                            .min_w(px(64.))
                            .text_size(metrics::TEXT_LABEL)
                            .text_color(t.muted_foreground)
                            .child(label),
                    )
                    .child(
                        div()
                            .id("find-match-case")
                            .h(px(22.))
                            .px(px(5.))
                            .flex()
                            .items_center()
                            .rounded(metrics::RADIUS_MD)
                            .text_size(metrics::TEXT_LABEL)
                            .font_weight(FontWeight::SEMIBOLD)
                            .cursor_pointer()
                            .text_color(if self.match_case { t.foreground } else { t.muted_foreground })
                            .when(self.match_case, |s| s.bg(t.primary.opacity(0.25)))
                            .when(!self.match_case, |s| s.hover(|s| s.bg(t.ghost_hover())))
                            .tooltip(|window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new("Match Case").build(window, cx)
                            })
                            .child("Aa")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.match_case = !this.match_case;
                                if !this.text(cx).is_empty() {
                                    this.search(true, cx);
                                }
                                cx.notify();
                            })),
                    )
                    .child(div().w(px(1.)).h(px(18.)).bg(t.border))
                    .child(
                        icon_button("find-prev", "chevron-up", px(26.), has_text, cx)
                            .on_click(cx.listener(|this, _, _, cx| this.step(false, cx))),
                    )
                    .child(
                        icon_button("find-next", "chevron-down", px(26.), has_text, cx)
                            .on_click(cx.listener(|this, _, _, cx| this.step(true, cx))),
                    )
                    .child(
                        icon_button("find-close", "x", px(26.), true, cx)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(FindBarEvent::Close))),
                    ),
            )
    }
}
