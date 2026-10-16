//! Toasts: short confirmations in the top-right corner of the web card,
//! like Zen's ("Copied current URL!"): a dark accent pill with bold text
//! that slides down into place and fades out.

use std::time::Duration;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::{
    browser::{Browser, TOAST_EXIT, ToastStyle},
    theme::{ActiveTokens, metrics},
    ui::components::{h_flex, icon, v_flex},
};

const TOAST_ENTER: Duration = Duration::from_millis(200);
/// Inset from the card's top-right corner.
const TOAST_INSET: Pixels = px(8.);
/// Extra top offset while the find bar occupies the corner.
const FIND_BAR_CLEARANCE: Pixels = px(52.);

impl Browser {
    /// The toast stack, positioned for the web card it is placed in.
    pub fn render_toasts(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.toasts.is_empty() {
            return None;
        }
        let t = cx.tokens().clone();
        let top = TOAST_INSET + if self.find_bar.is_some() { FIND_BAR_CLEARANCE } else { px(0.) };
        let toasts: Vec<AnyElement> = self
            .toasts
            .iter()
            .map(|toast| {
                let id = toast.id;
                let action = toast.action.clone();
                let glyph = match toast.style {
                    ToastStyle::Success => Some(("circle-check", t.status_success)),
                    ToastStyle::Warning => Some(("triangle-alert", t.status_warning)),
                    ToastStyle::Error => Some(("circle-x", t.destructive)),
                    ToastStyle::Info => None,
                };
                let pill = h_flex()
                    .id(("toast", id as usize))
                    .h(px(40.))
                    .px(px(14.))
                    .gap(px(10.))
                    .rounded(metrics::RADIUS)
                    // Accent tint over the popover colour: a dark blue-ish
                    // pill in dark mode, a light tint in light mode.
                    .bg(t.primary.opacity(if t.dark { 0.28 } else { 0.14 }))
                    .text_size(px(14.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.foreground)
                    .cursor_pointer()
                    .when_some(glyph, |s, (glyph, color)| s.child(icon(glyph, px(15.), color)))
                    .child(toast.text.clone())
                    .when_some(action, |s, (label, action)| {
                        s.child(
                            div()
                                .id(("toast-action", id as usize))
                                .px_2()
                                .py_0p5()
                                .rounded(metrics::RADIUS_MD)
                                .bg(t.foreground.opacity(0.12))
                                .hover(|s| s.bg(t.foreground.opacity(0.2)))
                                .child(label)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.run_toast_action(id, action.clone(), window, cx);
                                })),
                        )
                    })
                    .on_click(cx.listener(move |this, _, _, cx| this.dismiss_toast(id, cx)));
                let card = div()
                    .rounded(metrics::RADIUS)
                    .bg(t.popover)
                    .shadow(t.popover_shadow())
                    .child(pill);
                if toast.leaving {
                    card.with_animation(("toast-out", id as usize), Animation::new(TOAST_EXIT), |el, delta| {
                        el.opacity(1. - delta)
                    })
                    .into_any_element()
                } else {
                    card.with_animation(
                        ("toast-in", id as usize),
                        Animation::new(TOAST_ENTER).with_easing(ease_out_quint()),
                        |el, delta| el.opacity(delta).mt(px(-10.) * (1. - delta)),
                    )
                    .into_any_element()
                }
            })
            .collect();
        Some(
            v_flex()
                .absolute()
                .top(top)
                .right(TOAST_INSET)
                .items_end()
                .gap(px(6.))
                .children(toasts)
                .into_any_element(),
        )
    }
}
