//! GPUI view that shows one offscreen Chromium browser and feeds it input.
//!
//! Chromium paints BGRA frames (see `mori_engine::Frame`). The newest frame
//! is pulled only when this view actually renders, becomes a GPUI
//! `RenderImage` drawn into the view's bounds with the card's rounded
//! corners, and the image it replaces leaves the GPU atlas during that same
//! paint. A view that is not on screen therefore holds at most one pending
//! frame (in the engine) and uploads nothing. Mouse, wheel, keyboard, IME
//! and file drops are translated back into Chromium events.

use std::{ops::Range, sync::Arc};

use gpui_kit::*;
use mori_engine::{
    CursorKind, Frame, WebView,
    keys::{self, KeyInput, KeyPhase},
};
use smallvec::SmallVec;

/// Pixels scrolled per wheel "line" for devices that report lines.
const LINE_HEIGHT_PX: f32 = 50.0;

/// What the chrome needs to hear about from a page surface.
pub enum SurfaceEvent {
    /// A click or key press reached the page (a user gesture), so it may
    /// have just copied something.
    Interacted,
    /// The user pressed the paste shortcut; the chrome syncs the system
    /// clipboard into Chromium and then pastes.
    PasteRequested { plain: bool },
}

pub struct WebSurface {
    view: WebView,
    focus_handle: FocusHandle,
    image: Option<Arc<RenderImage>>,
    /// Size of `image` in device pixels.
    image_size: (u32, u32),
    /// Engine frame generation `image` was made from.
    generation: u64,
    /// Replaced images whose atlas tiles are released in the next paint
    /// (the frame on screen may still be drawing them until then).
    stale: Vec<Arc<RenderImage>>,
    popup: Option<(Arc<RenderImage>, Bounds<Pixels>)>,
    /// The `<select>` popup changed since it was last turned into an image.
    popup_dirty: bool,
    /// Files from outside are being dragged over the page.
    file_drag: bool,
    cursor: CursorKind,
    corner_radius: Pixels,
    /// Last laid-out bounds, window coordinates.
    bounds: Bounds<Pixels>,
    pressed: Option<mori_engine::MouseButton>,
    hovered: bool,
    marked_text: Option<String>,
    composition_bounds: Vec<Bounds<Pixels>>,
    had_focus: bool,
    /// Grayed out and non-interactive, e.g. behind a modal.
    pub inert: bool,
}

impl WebSurface {
    pub fn new(view: WebView, cx: &mut Context<Self>) -> Self {
        // Release this surface's GPU textures with it: the atlas only frees
        // images explicitly dropped.
        cx.on_release(|this: &mut Self, cx| {
            let images = this
                .image
                .take()
                .into_iter()
                .chain(this.popup.take().map(|(image, _)| image))
                .chain(this.stale.drain(..));
            for image in images {
                cx.drop_image(image, None);
            }
        })
        .detach();
        Self {
            view,
            focus_handle: cx.focus_handle(),
            image: None,
            image_size: (0, 0),
            generation: 0,
            stale: Vec::new(),
            popup: None,
            popup_dirty: false,
            file_drag: false,
            cursor: CursorKind::Arrow,
            corner_radius: px(10.),
            bounds: Bounds::default(),
            pressed: None,
            hovered: false,
            marked_text: None,
            composition_bounds: Vec::new(),
            had_focus: false,
            inert: false,
        }
    }

    pub fn view(&self) -> &WebView {
        &self.view
    }

    /// The current frame as `(width, height, BGRA bytes)`, device pixels.
    pub fn snapshot_bgra(&self) -> Option<(u32, u32, Vec<u8>)> {
        let image = self.image.as_ref()?;
        let bytes = image.as_bytes(0)?.to_vec();
        Some((self.image_size.0, self.image_size.1, bytes))
    }

    /// Bounds in window coordinates as of the last paint.
    pub fn bounds(&self) -> Bounds<Pixels> {
        self.bounds
    }

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus_handle
    }

    pub fn set_cursor(&mut self, cursor: CursorKind, cx: &mut Context<Self>) {
        if self.cursor != cursor {
            self.cursor = cursor;
            cx.notify();
        }
    }

    pub fn set_composition_bounds(&mut self, rects: &[(i32, i32, i32, i32)]) {
        let origin = self.bounds.origin;
        self.composition_bounds = rects
            .iter()
            .map(|&(x, y, w, h)| {
                Bounds::new(
                    point(origin.x + px(x as f32), origin.y + px(y as f32)),
                    size(px(w as f32), px(h as f32)),
                )
            })
            .collect();
    }

    /// Chromium painted: redraw if the frame is newer than the one shown.
    /// The frame itself is pulled in `render`, so frames painted while this
    /// view is not drawn are never copied or uploaded.
    pub fn sync_frame(&mut self, cx: &mut Context<Self>) {
        if self.view.frame_generation() != self.generation {
            cx.notify();
        }
    }

    pub fn sync_popup(&mut self, cx: &mut Context<Self>) {
        self.popup_dirty = true;
        cx.notify();
    }

    /// Turn the newest engine frame (and popup) into images; replaced
    /// images are queued for release in the coming paint.
    fn pull_frames(&mut self) {
        let generation = self.view.frame_generation();
        if generation != self.generation {
            self.generation = generation;
            if let Some(frame) = self.view.take_frame() {
                self.image_size = (frame.width, frame.height);
                if let Some(old) = self.image.replace(frame_to_image(frame)) {
                    self.stale.push(old);
                }
            }
        }
        if std::mem::take(&mut self.popup_dirty) {
            if let Some((old, _)) = self.popup.take() {
                self.stale.push(old);
            }
            if let Some(p) = self.view.popup()
                && p.frame.width > 0
            {
                let bounds = Bounds::new(
                    point(px(p.x as f32), px(p.y as f32)),
                    size(px(p.width as f32), px(p.height as f32)),
                );
                self.popup = Some((frame_to_image(p.frame), bounds));
            }
        }
    }

    /// Files dragged from outside moved over this surface.
    fn on_file_drag_move(&mut self, ev: &DragMoveEvent<ExternalPaths>, cx: &mut App) {
        if self.inert {
            return;
        }
        let position = ev.event.position;
        if !self.bounds.contains(&position) {
            if std::mem::take(&mut self.file_drag) {
                self.view.drag_files_leave();
            }
            return;
        }
        self.file_drag = true;
        let (x, y) = self.local(position);
        let mods = modifier_flags(&ev.event.modifiers);
        self.view.drag_files_over(ev.drag(cx).paths(), x, y, mods);
    }

    fn on_file_drop(&mut self, paths: &ExternalPaths, window: &mut Window) {
        self.file_drag = false;
        if self.inert {
            return;
        }
        let (x, y) = self.local(window.mouse_position());
        self.view.drop_files(paths.paths(), x, y, 0);
    }

    fn local(&self, position: Point<Pixels>) -> (i32, i32) {
        let p = position - self.bounds.origin;
        (f32::from(p.x).round() as i32, f32::from(p.y).round() as i32)
    }

    fn mouse_modifiers(&self, m: &Modifiers) -> u32 {
        let mut flags = modifier_flags(m);
        flags |= match self.pressed {
            Some(mori_engine::MouseButton::Left) => keys::MOD_LEFT_BUTTON,
            Some(mori_engine::MouseButton::Middle) => keys::MOD_MIDDLE_BUTTON,
            Some(mori_engine::MouseButton::Right) => keys::MOD_RIGHT_BUTTON,
            None => 0,
        };
        flags
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.inert {
            return;
        }
        let ks = &event.keystroke;
        let input = KeyInput {
            key: ks.key.clone(),
            phase: KeyPhase::Down,
            modifiers: modifier_flags(&ks.modifiers),
            repeat: event.is_held,
        };
        if !input.is_known() {
            return;
        }
        // Paste goes through the chrome so the system clipboard is used.
        let m = &ks.modifiers;
        let secondary = if cfg!(target_os = "macos") { m.platform } else { m.control };
        if secondary && !m.alt && ks.key == "v" {
            cx.emit(SurfaceEvent::PasteRequested { plain: m.shift });
            cx.stop_propagation();
            return;
        }
        self.view.send_key(&input);
        // Printable text without command modifiers comes back through the
        // input handler (IME-aware); everything else is fully handled here.
        let m = &ks.modifiers;
        let types_text = ks.key_char.is_some() && !m.control && !m.alt && !m.platform;
        if !types_text {
            cx.stop_propagation();
        }
    }

    fn on_key_up(&mut self, event: &KeyUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.inert {
            return;
        }
        cx.emit(SurfaceEvent::Interacted);
        let ks = &event.keystroke;
        self.view.send_key(&KeyInput {
            key: ks.key.clone(),
            phase: KeyPhase::Up,
            modifiers: modifier_flags(&ks.modifiers),
            repeat: false,
        });
    }

    fn paint_listeners(
        entity: Entity<Self>,
        hitbox: Hitbox,
        window: &mut Window,
    ) {
        let e = entity.clone();
        let hb = hitbox.clone();
        window.on_mouse_event(move |ev: &MouseDownEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || !hb.is_hovered(window) {
                return;
            }
            e.update(cx, |this, cx| {
                if this.inert {
                    return;
                }
                let Some(button) = map_button(ev.button) else { return };
                window.focus(&this.focus_handle, cx);
                this.view.set_focus(true);
                this.pressed = Some(button);
                let (x, y) = this.local(ev.position);
                let mods = this.mouse_modifiers(&ev.modifiers);
                this.view.send_mouse_click(x, y, button, false, ev.click_count as i32, mods);
            });
            cx.stop_propagation();
        });

        let e = entity.clone();
        window.on_mouse_event(move |ev: &MouseUpEvent, phase, _window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            e.update(cx, |this, cx| {
                let Some(button) = map_button(ev.button) else { return };
                if this.pressed != Some(button) {
                    return;
                }
                let (x, y) = this.local(ev.position);
                let mods = this.mouse_modifiers(&ev.modifiers);
                this.pressed = None;
                this.view.send_mouse_click(x, y, button, true, ev.click_count as i32, mods);
                cx.emit(SurfaceEvent::Interacted);
            });
        });

        let e = entity.clone();
        let hb = hitbox.clone();
        window.on_mouse_event(move |ev: &MouseMoveEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            let hovered = hb.is_hovered(window);
            // A drag of files from outside drives the page through drag events.
            if cx.has_active_drag() {
                return;
            }
            e.update(cx, |this, _cx| {
                if this.inert {
                    return;
                }
                let (x, y) = this.local(ev.position);
                let mods = this.mouse_modifiers(&ev.modifiers);
                if hovered || this.pressed.is_some() {
                    this.view.send_mouse_move(x, y, mods, false);
                } else if this.hovered {
                    this.view.send_mouse_move(x, y, mods, true);
                }
                this.hovered = hovered;
            });
        });

        let e = entity;
        let hb = hitbox;
        window.on_mouse_event(move |ev: &ScrollWheelEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || !hb.is_hovered(window) {
                return;
            }
            e.update(cx, |this, _cx| {
                if this.inert {
                    return;
                }
                let (dx, dy) = match ev.delta {
                    ScrollDelta::Pixels(p) => (f32::from(p.x), f32::from(p.y)),
                    ScrollDelta::Lines(l) => (l.x * LINE_HEIGHT_PX, l.y * LINE_HEIGHT_PX),
                };
                let (x, y) = this.local(ev.position);
                let mods = modifier_flags(&ev.modifiers);
                this.view
                    .send_mouse_wheel(x, y, dx.round() as i32, dy.round() as i32, mods);
            });
            cx.stop_propagation();
        });
    }
}

fn frame_to_image(frame: Frame) -> Arc<RenderImage> {
    let Frame { width, height, bgra } = frame;
    // RenderImage stores BGRA; the `RgbaImage` container is just a byte buffer
    // of the right shape here.
    let buffer = image::RgbaImage::from_raw(width, height, bgra)
        .unwrap_or_else(|| image::RgbaImage::new(width.max(1), height.max(1)));
    let frames: SmallVec<[image::Frame; 1]> = SmallVec::from_elem(image::Frame::new(buffer), 1);
    Arc::new(RenderImage::new(frames))
}

fn map_button(b: gpui_kit::MouseButton) -> Option<mori_engine::MouseButton> {
    match b {
        gpui_kit::MouseButton::Left => Some(mori_engine::MouseButton::Left),
        gpui_kit::MouseButton::Middle => Some(mori_engine::MouseButton::Middle),
        gpui_kit::MouseButton::Right => Some(mori_engine::MouseButton::Right),
        _ => None,
    }
}

pub fn modifier_flags(m: &Modifiers) -> u32 {
    let mut flags = 0;
    if m.shift {
        flags |= keys::MOD_SHIFT;
    }
    if m.control {
        flags |= keys::MOD_CONTROL;
    }
    if m.alt {
        flags |= keys::MOD_ALT;
    }
    if m.platform {
        flags |= keys::MOD_COMMAND;
    }
    flags
}

pub fn cursor_style(kind: CursorKind) -> CursorStyle {
    match kind {
        CursorKind::Arrow => CursorStyle::Arrow,
        CursorKind::Pointer => CursorStyle::PointingHand,
        CursorKind::Text => CursorStyle::IBeam,
        CursorKind::Crosshair => CursorStyle::Crosshair,
        CursorKind::Wait | CursorKind::Help => CursorStyle::Arrow,
        CursorKind::Move | CursorKind::Grab => CursorStyle::OpenHand,
        CursorKind::Grabbing => CursorStyle::ClosedHand,
        CursorKind::NotAllowed => CursorStyle::OperationNotAllowed,
        CursorKind::ResizeEw => CursorStyle::ResizeLeftRight,
        CursorKind::ResizeNs => CursorStyle::ResizeUpDown,
        CursorKind::ResizeNesw => CursorStyle::ResizeUpRightDownLeft,
        CursorKind::ResizeNwse => CursorStyle::ResizeUpLeftDownRight,
        CursorKind::ResizeCol => CursorStyle::ResizeColumn,
        CursorKind::ResizeRow => CursorStyle::ResizeRow,
        CursorKind::None => CursorStyle::Arrow,
    }
}

impl EventEmitter<SurfaceEvent> for WebSurface {}

impl Focusable for WebSurface {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for WebSurface {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Keep Chromium's focus in sync with GPUI's (caret blink, :focus).
        let focused = self.focus_handle.is_focused(window);
        if focused != self.had_focus {
            self.had_focus = focused;
            self.view.set_focus(focused);
            if !focused {
                self.view.send_capture_lost();
            }
        }

        self.pull_frames();
        let entity = cx.entity();
        let image = self.image.clone();
        let image_size = self.image_size;
        let popup = self.popup.clone();
        let stale = std::mem::take(&mut self.stale);
        let radius = self.corner_radius;
        let cursor = cursor_style(self.cursor);
        let focus_handle = self.focus_handle.clone();
        let inert = self.inert;

        div()
            .id("web-surface")
            .size_full()
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key_down))
            .on_key_up(cx.listener(Self::on_key_up))
            .on_drag_move({
                let entity = entity.clone();
                move |ev: &DragMoveEvent<ExternalPaths>, _, cx| {
                    entity.update(cx, |this, cx| this.on_file_drag_move(ev, cx));
                }
            })
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, _| {
                this.on_file_drop(paths, window)
            }))
            .child(
                canvas(
                    |bounds, window, _cx| window.insert_hitbox(bounds, HitboxBehavior::Normal),
                    move |bounds, hitbox, window, cx| {
                        for old in stale {
                            let _ = window.drop_image(old);
                        }
                        let scale = window.scale_factor();
                        entity.update(cx, |this, _| {
                            this.bounds = bounds;
                            let w = f32::from(bounds.size.width).round() as i32;
                            let h = f32::from(bounds.size.height).round() as i32;
                            this.view.resize(w, h, scale);
                        });

                        let corners = Corners::all(radius);
                        window.paint_quad(fill(bounds, gpui_kit::white()).corner_radii(corners));
                        if let Some(image) = image {
                            let natural = size(
                                px(image_size.0 as f32 / scale),
                                px(image_size.1 as f32 / scale),
                            );
                            let image_bounds = Bounds::new(bounds.origin, natural);
                            let _ = window.paint_image(bounds, image_bounds, corners, image, 0, inert);
                        }
                        if let Some((popup, rect)) = popup {
                            let rect = Bounds::new(bounds.origin + rect.origin, rect.size);
                            let _ = window.paint_image(rect, rect, Corners::default(), popup, 0, false);
                        }
                        window.set_cursor_style(cursor, &hitbox);
                        if focus_handle.is_focused(window) {
                            window.handle_input(
                                &focus_handle,
                                ElementInputHandler::new(bounds, entity.clone()),
                                cx,
                            );
                        }
                        Self::paint_listeners(entity, hitbox, window);
                    },
                )
                .size_full(),
            )
    }
}

impl EntityInputHandler for WebSurface {
    fn text_for_range(
        &mut self,
        _range: Range<usize>,
        _adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        None
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let len = self.marked_text.as_ref().map(|t| t.encode_utf16().count()).unwrap_or(0);
        Some(UTF16Selection {
            range: len..len,
            reversed: false,
        })
    }

    fn marked_text_range(&self, _window: &mut Window, _cx: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked_text
            .as_ref()
            .map(|t| 0..t.encode_utf16().count())
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        if self.marked_text.take().is_some() {
            self.view.ime_finish();
        }
    }

    fn replace_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        if self.inert {
            return;
        }
        if self.marked_text.take().is_some() {
            self.view.ime_commit(text);
        } else {
            self.view.insert_text(text);
        }
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        if self.inert {
            return;
        }
        if new_text.is_empty() {
            self.marked_text = None;
            self.view.ime_cancel();
            return;
        }
        self.marked_text = Some(new_text.to_string());
        self.view
            .ime_set_composition(new_text, new_selected_range.map(|r| r.end as u32));
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let rect = self
            .composition_bounds
            .get(range_utf16.start)
            .or_else(|| self.composition_bounds.last())
            .copied();
        Some(rect.unwrap_or(Bounds::new(element_bounds.origin, size(px(1.), px(18.)))))
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}
