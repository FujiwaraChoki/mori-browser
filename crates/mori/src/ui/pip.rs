//! Picture-in-Picture: a small floating window playing a tab's video.
//!
//! Chromium's own Picture-in-Picture window cannot appear for offscreen
//! pages, so Mori draws its own: the tab keeps painting while it is off
//! screen and the window shows its frames cropped to the video. It opens
//! when a page asks (`video.requestPictureInPicture()`, routed through the
//! page bridge), from the mini player, or by itself when a playing video's
//! tab leaves the screen ("auto Picture-in-Picture"). It closes when the
//! tab comes back on screen.

use std::time::Duration;

use gpui_kit::*;
use mori_core::{MediaCommand, scripts::pip_rect_script, workspace::TabId};

use crate::{
    browser::Browser,
    theme::{ActiveTokens, metrics},
    ui::components::icon,
    web_surface::WebSurface,
};

/// Initial window size (16:9).
const PIP_SIZE: Size<Pixels> = Size { width: px(400.), height: px(225.) };
/// Gap to the screen corner it opens in.
const PIP_MARGIN: Pixels = px(24.);
/// How often the video's place in the page is read again (it moves when
/// the page scrolls or relayouts).
const RECT_POLL: Duration = Duration::from_millis(400);
/// Polls without a video before the window closes itself.
const GONE_AFTER_POLLS: u32 = 4;

/// Where the video is in the page frame, device pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Deserialize)]
struct VideoRect {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    playing: bool,
}

/// The open Picture-in-Picture window, kept by the browser.
pub struct PipState {
    pub tab: TabId,
    window: AnyWindowHandle,
}

impl PipState {
    /// Close the window. Deferred: this may run inside that window's own
    /// event handler, while it can't be updated.
    pub fn close_window(&self, cx: &mut App) {
        let window = self.window;
        cx.defer(move |cx| {
            let _ = window.update(cx, |_, window, _| window.remove_window());
        });
    }
}

pub struct PipWindow {
    browser: WeakEntity<Browser>,
    tab: TabId,
    surface: Entity<WebSurface>,
    rect: Option<VideoRect>,
    /// Holds the window's keyboard focus so key presses reach
    /// [`PipWindow::forward_key`].
    focus: FocusHandle,
    /// Where the left button went down, until the pointer has moved far
    /// enough to drag the window (a press on a control stays a click).
    press: Option<Point<Pixels>>,
    _frames: Subscription,
    _poll: Task<()>,
}

impl PipWindow {
    fn new(browser: WeakEntity<Browser>, tab: TabId, surface: Entity<WebSurface>, cx: &mut Context<Self>) -> Self {
        // New page frames arrive through the surface.
        let frames = cx.observe(&surface, |_, _, cx| cx.notify());
        let view = surface.read(cx).view().clone();
        let poll = cx.spawn(async move |this, cx| {
            let mut first = true;
            let mut missing = 0;
            loop {
                let (tx, rx) = futures::channel::oneshot::channel();
                view.evaluate(&pip_rect_script(first), move |r| {
                    let _ = tx.send(r.ok().and_then(|v| serde_json::from_value::<VideoRect>(v).ok()));
                });
                first = false;
                let rect = rx.await.ok().flatten();
                missing = if rect.is_some() { 0 } else { missing + 1 };
                let alive = this.update(cx, |this, cx| {
                    if missing >= GONE_AFTER_POLLS {
                        // The video went away (the page navigated or removed it).
                        this.with_browser(cx, |b, _, cx| b.close_pip(false, cx));
                    } else if rect.is_some() && this.rect != rect {
                        this.rect = rect;
                        cx.notify();
                    }
                });
                if alive.is_err() || missing >= GONE_AFTER_POLLS {
                    break;
                }
                cx.background_executor().timer(RECT_POLL).await;
            }
        });
        PipWindow {
            browser,
            tab,
            surface,
            rect: None,
            focus: cx.focus_handle(),
            press: None,
            _frames: frames,
            _poll: poll,
        }
    }

    fn with_browser(&self, cx: &mut App, f: impl FnOnce(&mut Browser, &mut Window, &mut Context<Browser>) + 'static) {
        let Some(browser) = self.browser.upgrade() else { return };
        let handle = browser.read(cx).window_handle();
        let _ = handle.update(cx, |_, window, cx| browser.update(cx, |b, cx| f(b, window, cx)));
    }

    /// Compositors give a new window the keyboard (Wayland lets no client
    /// refuse it), so keys pressed while this window has it go on to the
    /// browser window, where the user was typing: switching tabs with the
    /// keyboard keeps working after auto Picture-in-Picture pops up.
    fn forward_key(&self, input: PlatformInput, cx: &mut App) {
        let Some(browser) = self.browser.upgrade() else { return };
        let handle = browser.read(cx).window_handle();
        // Outside this event's dispatch: the browser may update this window.
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, cx| match input {
                PlatformInput::KeyDown(down) => {
                    window.dispatch_keystroke(down.keystroke, cx);
                }
                other => {
                    window.dispatch_event(other, cx);
                }
            });
        });
    }
}

/// How far (in pixels, along both axes) the pointer moves with the button
/// held before the window follows.
const DRAG_THRESHOLD: f32 = 4.;

impl Render for PipWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens().clone();
        let (frame, stale) = self.surface.update(cx, |s, _| s.share_frame());
        let rect = self.rect;
        let playing = rect.is_none_or(|r| r.playing);
        let tab = self.tab;
        let control = |id: &'static str, glyph: &'static str, size: Pixels| {
            div()
                .id(id)
                .size(size)
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .cursor_pointer()
                // Reads on the black letterbox bars as well as on video.
                .bg(rgba(0x282828cc))
                .border_1()
                .border_color(white().opacity(0.18))
                .hover(|s| s.bg(rgba(0x3c3c3ce6)))
                .child(icon(glyph, (size * 0.5).round(), white()))
        };
        div()
            .id("pip")
            .group("pip")
            .size_full()
            .relative()
            .bg(black())
            .overflow_hidden()
            .rounded(metrics::RADIUS_MD)
            .border_1()
            .border_color(t.border.opacity(0.4))
            .track_focus(&self.focus)
            // Dragging moves the window. The move starts once the pointer
            // has travelled a little: started on the press, the compositor
            // would take the pointer and the controls would never see the
            // release that makes their click.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, _, _| this.press = Some(ev.position)),
            )
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, _| this.press = None))
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, window, _| {
                let Some(start) = this.press else { return };
                if ev.pressed_button != Some(MouseButton::Left) {
                    this.press = None;
                    return;
                }
                let d = ev.position - start;
                if f32::from(d.x).abs() + f32::from(d.y).abs() >= DRAG_THRESHOLD {
                    this.press = None;
                    window.start_window_move();
                }
            }))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                this.forward_key(PlatformInput::KeyDown(ev.clone()), cx);
            }))
            .on_key_up(cx.listener(|this, ev: &KeyUpEvent, _, cx| {
                this.forward_key(PlatformInput::KeyUp(ev.clone()), cx);
            }))
            .on_modifiers_changed(cx.listener(|this, ev: &ModifiersChangedEvent, _, cx| {
                this.forward_key(PlatformInput::ModifiersChanged(ev.clone()), cx);
            }))
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, cx| {
                        for old in stale {
                            cx.drop_image(old, Some(window));
                        }
                        let Some((image, (iw, ih))) = frame else { return };
                        let (iw, ih) = (iw as f32, ih as f32);
                        // The part of the page frame to show: the video, or
                        // the whole page until its place is known.
                        let r = rect
                            .map(|r| {
                                let x = r.x.clamp(0., iw);
                                let y = r.y.clamp(0., ih);
                                (x, y, r.width.min(iw - x), r.height.min(ih - y))
                            })
                            .filter(|r| r.2 >= 2. && r.3 >= 2.)
                            .unwrap_or((0., 0., iw, ih));
                        let bw = f32::from(bounds.size.width);
                        let bh = f32::from(bounds.size.height);
                        let k = (bw / r.2).min(bh / r.3);
                        let (dw, dh) = (r.2 * k, r.3 * k);
                        let dest = Bounds::new(
                            bounds.origin + point(px((bw - dw) / 2.), px((bh - dh) / 2.)),
                            size(px(dw), px(dh)),
                        );
                        let image_bounds = Bounds::new(
                            dest.origin - point(px(r.0 * k), px(r.1 * k)),
                            size(px(iw * k), px(ih * k)),
                        );
                        let _ = window.paint_image(dest, image_bounds, Corners::default(), image, 0, false);
                    },
                )
                .size_full(),
            )
            // Controls show while the pointer is over the window.
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .opacity(0.)
                    .group_hover("pip", |s| s.opacity(1.))
                    .bg(black().opacity(0.18))
                    .child(
                        div()
                            .absolute()
                            .top(px(8.))
                            .right(px(8.))
                            .flex()
                            .gap(px(6.))
                            .child(
                                control("pip-back", "arrow-up-left", px(28.)).on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.with_browser(cx, move |b, window, cx| {
                                            b.select_tab(tab, window, cx);
                                            window.activate_window();
                                        });
                                    },
                                )),
                            )
                            .child(control("pip-close", "x", px(28.)).on_click(cx.listener(|this, _, _, cx| {
                                this.with_browser(cx, |b, _, cx| b.close_pip(true, cx));
                            }))),
                    )
                    .child(
                        div().absolute().inset_0().flex().items_center().justify_center().child(
                            control("pip-play", if playing { "pause" } else { "play" }, px(44.)).on_click(
                                cx.listener(move |this, _, _, cx| {
                                    this.with_browser(cx, move |b, _, cx| {
                                        b.media_command(tab, MediaCommand::TogglePlay, cx)
                                    });
                                    if let Some(r) = this.rect.as_mut() {
                                        r.playing = !r.playing;
                                    }
                                    cx.notify();
                                }),
                            ),
                        ),
                    ),
            )
    }
}

impl Browser {
    /// The tab shown in Picture-in-Picture.
    pub fn pip_tab(&self) -> Option<TabId> {
        self.pip.as_ref().map(|p| p.tab)
    }

    /// Show `tab`'s video in a Picture-in-Picture window. `requested`: the
    /// page or the user asked (rather than auto Picture-in-Picture).
    /// Returns whether the window is open.
    pub fn open_pip(&mut self, tab: TabId, requested: bool, cx: &mut Context<Self>) -> bool {
        if self.pip_tab() == Some(tab) {
            return true;
        }
        let Some(live) = self.live.get(&tab) else { return false };
        if !requested && !self.media.get(&tab).is_some_and(|m| m.state.is_video && m.state.playing) {
            return false;
        }
        let surface = live.surface.clone();
        self.close_pip(false, cx);
        let browser = cx.entity().downgrade();
        let bounds = match cx.primary_display() {
            Some(display) => {
                let screen = display.bounds();
                Bounds::new(
                    screen.origin
                        + point(
                            screen.size.width - PIP_SIZE.width - PIP_MARGIN,
                            screen.size.height - PIP_SIZE.height - PIP_MARGIN,
                        ),
                    PIP_SIZE,
                )
            }
            None => Bounds::new(point(px(0.), px(0.)), PIP_SIZE),
        };
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                // Compositor rules written for Firefox's and Chrome's
                // Picture-in-Picture (float, pin) match this title.
                title: Some("Picture-in-Picture".into()),
                appears_transparent: true,
                traffic_light_position: None,
            }),
            focus: false,
            kind: WindowKind::Floating,
            is_movable: true,
            is_resizable: true,
            is_minimizable: false,
            app_id: Some("mori".into()),
            window_decorations: Some(WindowDecorations::Client),
            window_min_size: Some(size(px(200.), px(112.))),
            inactive_frame_interval: None,
            ..Default::default()
        };
        let opened = gpui_kit::open_window(options, cx, |window, cx| {
            let view = cx.new(|cx| PipWindow::new(browser.clone(), tab, surface, cx));
            let focus = view.read(cx).focus.clone();
            window.focus(&focus, cx);
            view
        });
        let (window, _) = match opened {
            Ok(opened) => opened,
            Err(e) => {
                log::warn!("opening Picture-in-Picture failed: {e:#}");
                return false;
            }
        };
        // The window manager closing it is like the close button.
        let weak = cx.entity().downgrade();
        let _ = window.update(cx, |_, window, cx| {
            window.on_window_should_close(cx, move |_, cx| {
                if let Some(b) = weak.upgrade() {
                    b.update(cx, |b, cx| {
                        b.pip = None;
                        b.after_pip_closed(tab, true, cx);
                    });
                }
                true
            });
        });
        self.pip = Some(PipState { tab, window });
        // Keep the page painting while it is off screen.
        if let Some(live) = self.live.get(&tab) {
            live.view.set_hidden(false);
        }
        cx.notify();
        true
    }

    /// Close Picture-in-Picture. `pause`: the user closed it (the video
    /// stops, as in Chrome), rather than returning to the tab.
    pub fn close_pip(&mut self, pause: bool, cx: &mut Context<Self>) {
        let Some(pip) = self.pip.take() else { return };
        pip.close_window(cx);
        self.after_pip_closed(pip.tab, pause, cx);
    }

    fn after_pip_closed(&mut self, tab: TabId, pause: bool, cx: &mut Context<Self>) {
        if let Some(live) = self.live.get(&tab) {
            live.view.bridge_event(&serde_json::json!({ "t": "pip-left" }), false);
            if pause && self.media.get(&tab).is_some_and(|m| m.state.playing) {
                self.media_command(tab, MediaCommand::TogglePlay, cx);
            }
        }
        self.sync_view_visibility(cx);
        cx.notify();
    }

    /// Keep Picture-in-Picture in step with the tabs on screen: a tab that
    /// comes back closes it, a playing video leaving opens it (auto
    /// Picture-in-Picture), and a tab that is gone takes it along.
    pub fn sync_pip(&mut self, left: &[TabId], cx: &mut Context<Self>) {
        let visible = [self.selected(), self.split];
        if let Some(tab) = self.pip_tab()
            && (visible.contains(&Some(tab)) || !self.live.contains_key(&tab))
        {
            self.close_pip(false, cx);
        }
        if self.settings.auto_pip
            && self.pip.is_none()
            && let Some(&tab) = left.iter().find(|t| !visible.contains(&Some(**t)))
        {
            self.open_pip(tab, false, cx);
        }
    }
}
