//! Peek: a link previewed in a large card over the page, like Zen's Glance
//! and Arc's Peek. The page behind dims, the sidebar stays as it is, and a
//! column of round buttons beside the card closes the preview, opens it as a
//! tab, or opens it in a split.

use std::time::Duration;

use gpui_kit::*;

use crate::{
    browser::{Browser, PageOwner},
    theme::{ActiveTokens, metrics},
    ui::components::{icon, v_flex},
};

/// Share of the content area the card covers horizontally.
const CARD_WIDTH_RATIO: f32 = 0.8;
const CARD_MAX_WIDTH: f32 = 1200.;
/// Room kept right of the card for the control column.
const CONTROLS_GUTTER: f32 = 56.;
const CONTROL_SIZE: Pixels = px(32.);

impl Browser {
    /// The area pages are shown in (both halves of a split), in window
    /// coordinates, as of the last paint.
    pub(crate) fn peek_area(&self, window: &Window, cx: &App) -> Bounds<Pixels> {
        let painted = [self.selected(), self.split]
            .into_iter()
            .flatten()
            .filter_map(|id| self.live.get(&id))
            .map(|l| l.surface.read(cx).bounds())
            .filter(|b| b.size.width > px(0.) && b.size.height > px(0.))
            .reduce(|a, b| a.union(&b));
        painted.unwrap_or_else(|| {
            // Nothing painted yet (settings, new-tab page): the card column.
            let viewport = window.viewport_size();
            let inset = metrics::CARD_INSET;
            let sidebar = if self.settings.sidebar_visible {
                px(self.settings.clamped_sidebar_width()) + inset
            } else {
                px(0.)
            };
            let left_sidebar = self.settings.sidebar_position == mori_core::settings::SidebarPosition::Left;
            let (left, right) = if left_sidebar { (sidebar.max(inset), inset) } else { (inset, sidebar.max(inset)) };
            Bounds::new(
                point(left, inset),
                size(viewport.width - left - right, viewport.height - inset * 2.),
            )
        })
    }

    /// Card bounds for a peek shown over `area`.
    fn peek_card(area: Bounds<Pixels>) -> Bounds<Pixels> {
        let room = (f32::from(area.size.width) - CONTROLS_GUTTER * 2.).max(200.);
        let width = (f32::from(area.size.width) * CARD_WIDTH_RATIO).min(CARD_MAX_WIDTH).min(room);
        let left = f32::from(area.left()) + (f32::from(area.size.width) - width) / 2.;
        Bounds::new(point(px(left), area.top()), size(px(width), area.size.height))
    }

    /// Initial size for a peeked page's view, in logical pixels.
    pub(crate) fn peek_view_size(&self, window: &Window, cx: &App) -> (i32, i32) {
        let card = Self::peek_card(self.peek_area(window, cx));
        (f32::from(card.size.width) as i32, f32::from(card.size.height) as i32)
    }

    pub fn render_peek(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let peek = self.peek.as_ref()?;
        let t = cx.tokens().clone();
        let area = self.peek_area(window, cx);
        let card = Self::peek_card(area);
        let key = peek.view.id().0 as usize;
        let surface = peek.surface.clone();
        // Dialogs, sign-in and permission prompts of the peeked page.
        let prompts = self.render_page_prompts(PageOwner::Peek, cx);

        let control = |id: &'static str, glyph: &'static str, tip: &'static str| {
            div()
                .id(id)
                .size(CONTROL_SIZE)
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(t.popover)
                .border_1()
                .border_color(white().opacity(if t.dark { 0.10 } else { 0.0 }))
                .shadow(t.popover_shadow())
                .cursor_pointer()
                .child(
                    div()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(t.primary.opacity(0.30))
                        .hover(|s| s.bg(t.primary.opacity(0.45)))
                        .child(icon(glyph, px(15.), t.foreground)),
                )
                .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(tip).build(window, cx))
        };
        let controls = v_flex()
            .absolute()
            .left(card.right() + px(14.))
            .top(card.top() + px(14.))
            .gap(px(12.))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                control("peek-close", "x", "Close")
                    .on_click(cx.listener(|this, _, window, cx| this.close_peek(window, cx))),
            )
            .child(
                control("peek-open", "maximize-2", "Open as Tab")
                    .on_click(cx.listener(|this, _, window, cx| this.promote_peek(window, cx))),
            )
            .child(
                control("peek-split", "columns-2", "Open in Split View")
                    .on_click(cx.listener(|this, _, window, cx| this.peek_to_split(window, cx))),
            )
            .with_animation(
                ("peek-controls", key),
                Animation::new(Duration::from_millis(240)).with_easing(ease_out_quint()),
                |el, delta| el.opacity(delta),
            );

        let card_el = div()
            .absolute()
            .left(card.left())
            .top(card.top())
            .w(card.size.width)
            .h(card.size.height)
            .rounded(metrics::RADIUS)
            .bg(t.card)
            .border_1()
            .border_color(t.border.opacity(0.7))
            .shadow(vec![BoxShadow {
                color: black().opacity(t.shadow_alpha(0.25, 0.5)),
                offset: point(px(0.), px(12.)),
                blur_radius: px(48.),
                spread_radius: px(0.),
                inset: false,
            }])
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(surface)
            .children(prompts)
            .with_animation(
                ("peek-card", key),
                Animation::new(Duration::from_millis(220)).with_easing(ease_out_quint()),
                move |el, delta| el.opacity(delta).top(card.top() + px(24.) * (1. - delta)),
            );

        let backdrop = div()
            .absolute()
            .left(area.left())
            .top(area.top())
            .w(area.size.width)
            .h(area.size.height)
            .rounded(metrics::RADIUS)
            .bg(black().opacity(if t.dark { 0.6 } else { 0.45 }))
            .with_animation(
                ("peek-dim", key),
                Animation::new(Duration::from_millis(180)),
                |el, delta| el.opacity(delta),
            );

        Some(
            div()
                .absolute()
                .inset_0()
                .occlude()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| this.close_peek(window, cx)),
                )
                .child(backdrop)
                .child(card_el)
                .child(controls)
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::{Bounds, point, px, size};

    use crate::browser::Browser;

    #[test]
    fn card_is_centred_with_room_for_controls() {
        // A 1246px content area (Zen at 1440px): ~80% wide, full height.
        let area = Bounds::new(point(px(186.), px(8.)), size(px(1246.), px(884.)));
        let card = Browser::peek_card(area);
        assert_eq!(card.size.width, px(996.8));
        assert_eq!(card.size.height, px(884.));
        assert_eq!(card.top(), px(8.));
        assert_eq!(card.left(), px(186. + (1246. - 996.8) / 2.));
        // Narrow areas keep a gutter for the control column on both sides.
        let narrow = Bounds::new(point(px(0.), px(0.)), size(px(400.), px(600.)));
        assert_eq!(Browser::peek_card(narrow).size.width, px(288.));
    }
}
