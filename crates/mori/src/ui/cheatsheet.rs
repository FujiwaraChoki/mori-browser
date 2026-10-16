//! Keyboard shortcut cheatsheet (Ctrl+/): every default binding, grouped
//! and searchable, built from the key binding registry in `actions.rs` so
//! it never drifts from the real keys.

use std::time::Duration;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    component::input::{Escape, Input, InputEvent, InputState},
    *,
};
use mori_core::cheatsheet::{self, Entry};

use crate::{
    browser::Browser,
    theme::{ActiveTokens, metrics},
    ui::components::{h_flex, icon, v_flex},
};

pub enum CheatsheetEvent {
    Dismiss,
}

pub struct Cheatsheet {
    input: Entity<InputState>,
    entries: Vec<Entry>,
    _subscription: Subscription,
}

impl EventEmitter<CheatsheetEvent> for Cheatsheet {}

impl Cheatsheet {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search shortcuts"));
        let sub = cx.subscribe_in(&input, window, |_, _, ev: &InputEvent, _, cx| {
            if matches!(ev, InputEvent::Change) {
                cx.notify();
            }
        });
        Cheatsheet {
            input,
            entries: crate::actions::cheatsheet_entries(),
            _subscription: sub,
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |s, cx| s.focus(window, cx));
    }
}

/// Key caps for a label like "Ctrl+Shift+T" (a trailing "+" is the key).
fn caps(label: &str, cx: &App) -> Div {
    let t = cx.tokens();
    let mut parts: Vec<&str> = match label.strip_suffix("++") {
        Some(mods) => mods.split('+').chain(["+"]).collect(),
        None => label.split('+').collect(),
    };
    parts.retain(|p| !p.is_empty());
    h_flex().gap_0p5().flex_none().children(parts.into_iter().map(|k| {
        div()
            .min_w(px(20.))
            .h(px(20.))
            .px_1()
            .flex()
            .items_center()
            .justify_center()
            .rounded(metrics::RADIUS_MD)
            .bg(t.foreground.opacity(0.07))
            .border_1()
            .border_color(t.foreground.opacity(0.06))
            .text_size(metrics::TEXT_CAPTION)
            .text_color(t.muted_foreground)
            .child(k.to_string())
    }))
}

/// A row's keys: one or two bindings side by side; a run of numbered ones
/// (Ctrl+1 … Ctrl+8) as its first and last.
fn keys_of(keys: &[String], cx: &App) -> Div {
    let t = cx.tokens();
    let row = h_flex().gap(px(6.)).flex_none();
    match keys {
        [first, .., last] if keys.len() > 2 => row
            .child(caps(first, cx))
            .child(div().text_size(metrics::TEXT_SMALL).text_color(t.muted_foreground).child("…"))
            .child(caps(last, cx)),
        _ => row.children(keys.iter().map(|k| caps(k, cx))),
    }
}

impl Render for Cheatsheet {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens().clone();
        let viewport = window.viewport_size();
        let width = px(760.).min(viewport.width - px(32.));
        let top = (viewport.height * 0.12).max(px(16.));
        let max_height = (viewport.height - top * 2.).max(px(200.));
        let query = self.input.read(cx).value().to_string();
        let groups = cheatsheet::grouped(&self.entries, &query);
        let (panel_bg, border): (Hsla, Hsla) = if t.dark {
            (rgb(0x181818).into(), rgb(0x4b4b4b).into())
        } else {
            (white(), t.popover_border())
        };
        let separator = t.foreground.opacity(if t.dark { 0.08 } else { 0.07 });

        let group_cards: Vec<AnyElement> = groups
            .into_iter()
            .map(|(name, entries)| {
                v_flex()
                    .gap(px(2.))
                    .child(
                        div()
                            .px(px(8.))
                            .pb(px(4.))
                            .text_size(metrics::TEXT_SMALL)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(t.muted_foreground)
                            .child(name.to_uppercase()),
                    )
                    .children(entries.into_iter().map(|e| {
                        h_flex()
                            .min_h(px(30.))
                            .px(px(8.))
                            .gap_3()
                            .rounded(metrics::RADIUS_LG)
                            .hover(|s| s.bg(t.ghost_hover()))
                            .child(div().flex_1().min_w_0().truncate().child(e.title.clone()))
                            .child(keys_of(&e.keys, cx))
                    }))
                    .into_any_element()
            })
            .collect();
        let empty = group_cards.is_empty();
        // Two balanced columns of sections.
        let mut left = Vec::new();
        let mut right = Vec::new();
        for (i, card) in group_cards.into_iter().enumerate() {
            if i % 2 == 0 { left.push(card) } else { right.push(card) }
        }

        let panel = v_flex()
            .id("cheatsheet")
            .absolute()
            .top(top)
            .left((viewport.width - width) / 2.)
            .w(width)
            .max_h(max_height)
            .bg(panel_bg)
            .rounded(metrics::RADIUS)
            .border_1()
            .border_color(border)
            .shadow(t.modal_shadow())
            .text_color(t.foreground)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .capture_action(cx.listener(|_, _: &Escape, _, cx| {
                cx.emit(CheatsheetEvent::Dismiss);
                cx.stop_propagation();
            }))
            .child(
                h_flex()
                    .h(px(54.))
                    .flex_none()
                    .px(px(18.))
                    .gap(px(12.))
                    .child(icon("keyboard", px(16.), t.foreground.opacity(0.55)))
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(&self.input)
                                .appearance(false)
                                .bordered(false)
                                .focus_bordered(false)
                                .px_0()
                                .text_size(px(15.))
                                .line_height(px(22.)),
                        ),
                    )
                    .child(
                        div()
                            .text_size(metrics::TEXT_SMALL)
                            .text_color(t.muted_foreground)
                            .child("Keyboard Shortcuts"),
                    ),
            )
            .child(div().mx(px(10.)).h(px(1.)).bg(separator))
            .child(
                div()
                    .id("cheatsheet-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p(px(10.))
                    .when(empty, |s| {
                        s.child(
                            div()
                                .py(px(28.))
                                .text_center()
                                .text_color(t.muted_foreground)
                                .child(format!("No shortcut matches “{}”", query.trim())),
                        )
                    })
                    .when(!empty, |s| {
                        s.child(
                            h_flex()
                                .items_start()
                                .gap(px(14.))
                                .child(v_flex().flex_1().min_w_0().gap(px(14.)).children(left))
                                .child(v_flex().flex_1().min_w_0().gap(px(14.)).children(right)),
                        )
                    }),
            )
            .with_animation(
                "cheatsheet-open",
                Animation::new(Duration::from_millis(160)).with_easing(ease_out_quint()),
                move |el, delta| el.opacity(delta).top(top - px(8.) * (1. - delta)),
            );

        div()
            .absolute()
            .inset_0()
            .occlude()
            .on_mouse_down(MouseButton::Left, cx.listener(|_, _, _, cx| cx.emit(CheatsheetEvent::Dismiss)))
            .child(panel)
    }
}

impl Browser {
    /// Ctrl+/: show the cheatsheet, or close it if it is up.
    pub fn toggle_cheatsheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.cheatsheet.is_some() {
            self.close_cheatsheet(window, cx);
            return;
        }
        self.menu = None;
        if self.launcher.is_some() {
            self.close_launcher(window, cx);
        }
        let sheet = cx.new(|cx| Cheatsheet::new(window, cx));
        let sub = cx.subscribe_in(&sheet, window, |this, _, ev: &CheatsheetEvent, window, cx| match ev {
            CheatsheetEvent::Dismiss => this.close_cheatsheet(window, cx),
        });
        sheet.update(cx, |s, cx| s.focus(window, cx));
        self.cheatsheet = Some((sheet, sub));
        cx.notify();
    }

    pub fn close_cheatsheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.cheatsheet.take().is_some() {
            self.focus_selected_page(window, cx);
            cx.notify();
        }
    }

    pub fn render_cheatsheet(&self) -> Option<AnyElement> {
        self.cheatsheet.as_ref().map(|(sheet, _)| sheet.clone().into_any_element())
    }
}
