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
/// Extra top offset while the settings page shows: its header (and Done
/// button) is 56px tall.
const SETTINGS_HEADER_CLEARANCE: Pixels = px(56.);

impl Browser {
    /// The toast stack, positioned for the web card it is placed in.
    pub fn render_toasts(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.toasts.is_empty() {
            return None;
        }
        let t = cx.tokens().clone();
        let find_bar_shown = self.find_bar.is_some() && !self.settings_open;
        let top = TOAST_INSET
            + if self.settings_open {
                SETTINGS_HEADER_CLEARANCE
            } else if find_bar_shown {
                FIND_BAR_CLEARANCE
            } else {
                px(0.)
            };
        let toasts: Vec<AnyElement> = self
            .toasts
            .iter()
            .map(|toast| {
                let id = toast.id;
                let action = toast.action.clone();
                // Zen's toasts carry text only; a glyph marks trouble.
                let glyph = match toast.style {
                    ToastStyle::Warning => Some(("triangle-alert", t.status_warning)),
                    ToastStyle::Error => Some(("circle-x", t.destructive)),
                    ToastStyle::Success | ToastStyle::Info => None,
                };
                // Zen's pill: a vertical navy gradient with a light outline
                // in dark mode, a saturated blue one in light mode, white text.
                let (top_color, bottom_color, outline): (Hsla, Hsla, Hsla) = if t.dark {
                    (rgb(0x132a47).into(), rgb(0x122640).into(), white().opacity(0.65))
                } else {
                    (rgb(0x1a78f0).into(), rgb(0x1969d0).into(), white().opacity(0.35))
                };
                let pill = h_flex()
                    .id(("toast", id as usize))
                    .h(px(40.))
                    .px(px(14.))
                    .gap(px(10.))
                    .rounded(metrics::RADIUS)
                    .bg(linear_gradient(
                        180.,
                        linear_color_stop(top_color, 0.),
                        linear_color_stop(bottom_color, 1.),
                    ))
                    .border_1()
                    .border_color(outline)
                    .text_size(px(14.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(white())
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
                                .bg(white().opacity(0.14))
                                .hover(|s| s.bg(white().opacity(0.24)))
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
