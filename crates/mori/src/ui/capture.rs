//! Capture mode (Arc's Capture, Ctrl+Shift+X): the page dims, dragging a
//! rectangle captures that area, and a toolbar over the page captures the
//! visible area or the whole page instead. Captures are copied to the
//! clipboard or saved to Downloads, as chosen in the toolbar.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use mori_core::{capture::Region, workspace::TabId};

use crate::{
    browser::{Browser, ToastStyle},
    screenshot::Destination,
    theme::{ActiveTokens, metrics},
    ui::components::{h_flex, icon},
};

/// Capture mode on a page.
pub struct CaptureMode {
    tab: TabId,
    /// Drag start and current pointer, window coordinates.
    drag: Option<(Point<Pixels>, Point<Pixels>)>,
}

impl Browser {
    /// Enter capture mode on the active page (the focused split pane).
    pub fn start_capture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.active_tab().filter(|t| self.live.contains_key(t)) else {
            self.toast("Open a web page to capture it", ToastStyle::Info, None, cx);
            return;
        };
        if self.settings_open {
            self.set_settings_open(false, window, cx);
        }
        self.menu = None;
        if self.launcher.is_some() {
            self.close_launcher(window, cx);
        }
        self.capture = Some(CaptureMode { tab, drag: None });
        // Escape reaches the window, not the page.
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    pub fn cancel_capture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.capture.take().is_some() {
            self.focus_selected_page(window, cx);
            cx.notify();
        }
    }

    fn capture_area(&self, cx: &App) -> Option<(TabId, Bounds<Pixels>)> {
        let mode = self.capture.as_ref()?;
        let bounds = self.live.get(&mode.tab)?.surface.read(cx).bounds();
        (bounds.size.width > px(0.)).then_some((mode.tab, bounds))
    }

    fn finish_capture_drag(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((tab, bounds)) = self.capture_area(cx) else { return };
        let Some((start, end)) = self.capture.as_mut().and_then(|m| m.drag.take()) else { return };
        let local = |p: Point<Pixels>| (f32::from(p.x - bounds.origin.x), f32::from(p.y - bounds.origin.y));
        let size = (f32::from(bounds.size.width), f32::from(bounds.size.height));
        match Region::from_drag(local(start), local(end), size) {
            Some(region) => {
                let destination = self.capture_destination;
                self.cancel_capture(window, cx);
                self.capture_frame(tab, Some(region), &[destination], cx);
            }
            // A click without a drag: keep waiting for one.
            None => cx.notify(),
        }
    }

    pub fn render_capture(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (tab, bounds) = self.capture_area(cx)?;
        let drag = self.capture.as_ref().and_then(|m| m.drag);
        let shade = black().opacity(0.38);

        // The page dims except for the selection.
        let mut layers: Vec<AnyElement> = Vec::new();
        let page = (bounds.left(), bounds.top(), bounds.right(), bounds.bottom());
        let rect = |l: Pixels, top: Pixels, r: Pixels, b: Pixels| {
            div()
                .absolute()
                .left(l)
                .top(top)
                .w((r - l).max(px(0.)))
                .h((b - top).max(px(0.)))
        };
        match drag {
            Some((a, b)) => {
                let clamp_x = |v: Pixels| v.clamp(page.0, page.2);
                let clamp_y = |v: Pixels| v.clamp(page.1, page.3);
                let (l, r) = (clamp_x(a.x.min(b.x)), clamp_x(a.x.max(b.x)));
                let (top, bottom) = (clamp_y(a.y.min(b.y)), clamp_y(a.y.max(b.y)));
                layers.push(rect(page.0, page.1, page.2, top).bg(shade).into_any_element());
                layers.push(rect(page.0, bottom, page.2, page.3).bg(shade).into_any_element());
                layers.push(rect(page.0, top, l, bottom).bg(shade).into_any_element());
                layers.push(rect(r, top, page.2, bottom).bg(shade).into_any_element());
                layers.push(
                    rect(l, top, r, bottom)
                        .border_1()
                        .border_color(white().opacity(0.95))
                        .shadow(vec![BoxShadow {
                            color: black().opacity(0.35),
                            offset: point(px(0.), px(0.)),
                            blur_radius: px(0.),
                            spread_radius: px(1.),
                            inset: false,
                        }])
                        .into_any_element(),
                );
                let (w, h) = (f32::from(r - l).round(), f32::from(bottom - top).round());
                layers.push(
                    div()
                        .absolute()
                        .left(l)
                        .top((bottom + px(6.)).min(page.3 - px(24.)))
                        .px(px(6.))
                        .py(px(2.))
                        .rounded(metrics::RADIUS_MD)
                        .bg(black().opacity(0.72))
                        .text_size(metrics::TEXT_SMALL)
                        .text_color(white())
                        .child(format!("{w} × {h}"))
                        .into_any_element(),
                );
            }
            None => layers.push(
                rect(page.0, page.1, page.2, page.3)
                    .bg(shade)
                    .rounded(metrics::CARD_RADIUS)
                    .into_any_element(),
            ),
        }

        let toolbar = self.render_capture_toolbar(tab, cx);
        Some(
            div()
                .id("capture-mode")
                .absolute()
                .inset_0()
                .occlude()
                .cursor_crosshair()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                        if let Some(mode) = this.capture.as_mut() {
                            mode.drag = Some((ev.position, ev.position));
                            cx.notify();
                        }
                    }),
                )
                .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                    if let Some(mode) = this.capture.as_mut()
                        && let Some((_, end)) = mode.drag.as_mut()
                        && ev.dragging()
                    {
                        *end = ev.position;
                        cx.notify();
                    }
                }))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| this.finish_capture_drag(window, cx)),
                )
                .on_mouse_up_out(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| this.finish_capture_drag(window, cx)),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|this, _, window, cx| this.cancel_capture(window, cx)),
                )
                .children(layers)
                .child(
                    h_flex()
                        .absolute()
                        .left(bounds.left())
                        .w(bounds.size.width)
                        .top(bounds.top() + px(12.))
                        .justify_center()
                        .child(toolbar),
                )
                .into_any_element(),
        )
    }

    fn render_capture_toolbar(&mut self, tab: TabId, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.tokens().clone();
        let destination = self.capture_destination;
        let segment = |id: &'static str, label: &'static str, glyph: &'static str, value: Destination, cx: &mut Context<Self>| {
            let active = destination == value;
            h_flex()
                .id(id)
                .h(px(26.))
                .px(px(10.))
                .gap(px(6.))
                .rounded(metrics::RADIUS_LG)
                .cursor_pointer()
                .text_size(metrics::TEXT_LABEL)
                .when(active, |s| {
                    s.bg(if t.dark { white().opacity(0.16) } else { white() })
                        .text_color(t.foreground)
                        .font_weight(FontWeight::MEDIUM)
                        .when(!t.dark, |s| s.shadow_sm())
                })
                .when(!active, |s| s.text_color(t.foreground.opacity(0.7)).hover(|s| s.bg(t.ghost_hover())))
                .child(icon(glyph, px(13.), if active { t.foreground } else { t.muted_foreground }))
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.capture_destination = value;
                    cx.notify();
                }))
        };
        let action = |id: &'static str, label: &'static str, glyph: &'static str| {
            h_flex()
                .id(id)
                .h(px(28.))
                .px(px(10.))
                .gap(px(6.))
                .rounded(metrics::RADIUS_LG)
                .cursor_pointer()
                .text_size(metrics::TEXT_LABEL)
                .text_color(t.foreground)
                .hover(|s| s.bg(t.ghost_hover()))
                .child(icon(glyph, px(14.), t.foreground.opacity(0.8)))
                .child(label)
        };
        h_flex()
            .id("capture-toolbar")
            .h(px(40.))
            .px(px(6.))
            .gap(px(4.))
            .bg(t.popover)
            .rounded(metrics::RADIUS_POPOVER)
            .border_1()
            .border_color(t.popover_border())
            .shadow(t.overlay_shadow())
            .text_color(t.foreground)
            .cursor_default()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .px(px(8.))
                    .text_size(metrics::TEXT_LABEL)
                    .text_color(t.muted_foreground)
                    .child("Drag to capture an area"),
            )
            .child(
                h_flex()
                    .gap_0p5()
                    .p_0p5()
                    .rounded(px(8.))
                    .bg(t.foreground.opacity(0.06))
                    .child(segment("capture-copy", "Copy", "copy", Destination::Clipboard, cx))
                    .child(segment("capture-save", "Save", "download", Destination::Downloads, cx)),
            )
            .child(div().w(px(1.)).h(px(20.)).mx(px(2.)).bg(t.popover_separator()))
            .child(action("capture-visible", "Visible", "scan").on_click(cx.listener(move |this, _, window, cx| {
                let destination = this.capture_destination;
                this.cancel_capture(window, cx);
                this.capture_frame(tab, None, &[destination], cx);
            })))
            .child(action("capture-full", "Full Page", "scan-line").on_click(cx.listener(move |this, _, window, cx| {
                let destination = this.capture_destination;
                this.cancel_capture(window, cx);
                this.capture_full_page(tab, destination, cx);
            })))
            .child(
                div()
                    .id("capture-close")
                    .size(px(28.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(metrics::RADIUS_LG)
                    .cursor_pointer()
                    .hover(|s| s.bg(t.ghost_hover()))
                    .child(icon("x", px(14.), t.muted_foreground))
                    .on_click(cx.listener(|this, _, window, cx| this.cancel_capture(window, cx))),
            )
    }
}
