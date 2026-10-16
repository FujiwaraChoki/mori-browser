//! HTML5 drag and drop for offscreen views.
//!
//! Chromium cannot run a native drag session out of a windowless browser,
//! so drags stay inside the view, the way cefclient's offscreen mode does
//! it: when a page starts a drag (`start_dragging`), the view becomes its
//! own drop target and the pointer drives `drag_target_*` until the button
//! is released. Files dragged in from the desktop take the same path with
//! drag data built from their paths.

use std::{path::PathBuf, rc::Rc};

use cef::*;

use crate::view::{CursorKind, ViewEvent, ViewInner};

/// A drag in progress over a view.
pub(crate) struct DragSession {
    /// Operations the source allows.
    allowed: DragOperationsMask,
    /// What the target last said it would do (`update_drag_cursor`).
    current: DragOperationsMask,
    /// Started by the page itself (it must hear that the drag ended), as
    /// opposed to files dragged in from outside.
    from_page: bool,
}

fn op(mask: cef::sys::cef_drag_operations_mask_t) -> DragOperationsMask {
    mask.into()
}

fn none() -> DragOperationsMask {
    op(cef::sys::cef_drag_operations_mask_t::DRAG_OPERATION_NONE)
}

fn bits(mask: DragOperationsMask) -> u32 {
    let raw: cef::sys::cef_drag_operations_mask_t = mask.into();
    raw.0
}

fn inside(view: &ViewInner, x: i32, y: i32) -> bool {
    let (w, h) = view.size.get();
    (0..w).contains(&x) && (0..h).contains(&y)
}

/// `RenderHandler::start_dragging`: the page began dragging `data`.
pub(crate) fn start(
    view: &Rc<ViewInner>,
    data: &mut DragData,
    allowed: DragOperationsMask,
    x: i32,
    y: i32,
) -> bool {
    let Some(host) = view.host() else { return false };
    *view.drag.borrow_mut() = Some(DragSession {
        allowed,
        current: none(),
        from_page: true,
    });
    let event = MouseEvent { x, y, modifiers: 0 };
    host.drag_target_drag_enter(Some(data), Some(&event), allowed);
    host.drag_target_drag_over(Some(&event), allowed);
    true
}

/// `RenderHandler::update_drag_cursor`: the target accepts `operation`.
pub(crate) fn update_cursor(view: &Rc<ViewInner>, operation: DragOperationsMask) {
    let accepted = {
        let mut drag = view.drag.borrow_mut();
        let Some(session) = drag.as_mut() else { return };
        session.current = operation;
        bits(operation) != 0
    };
    view.push(ViewEvent::CursorChanged(if accepted {
        CursorKind::Grabbing
    } else {
        CursorKind::NotAllowed
    }));
}

/// Pointer moved during a page drag; returns whether a drag consumed it.
pub(crate) fn drag_move(view: &Rc<ViewInner>, x: i32, y: i32, modifiers: u32) -> bool {
    let allowed = match view.drag.borrow().as_ref() {
        Some(session) if session.from_page => session.allowed,
        _ => return false,
    };
    if let Some(host) = view.host() {
        let event = MouseEvent { x, y, modifiers };
        host.drag_target_drag_over(Some(&event), allowed);
    }
    true
}

/// Left button released during a page drag: drop it (or cancel it when
/// released outside the view). Returns whether a drag consumed the release.
pub(crate) fn drag_drop(view: &Rc<ViewInner>, x: i32, y: i32, modifiers: u32) -> bool {
    let session = match view.drag.borrow_mut().take() {
        Some(session) if session.from_page => session,
        Some(other) => {
            *view.drag.borrow_mut() = Some(other);
            return false;
        }
        None => return false,
    };
    let Some(host) = view.host() else { return true };
    let event = MouseEvent { x, y, modifiers };
    let operation = if inside(view, x, y) && bits(session.current) != 0 {
        host.drag_target_drop(Some(&event));
        session.current
    } else {
        host.drag_target_drag_leave();
        none()
    };
    host.drag_source_ended_at(x, y, operation);
    host.drag_source_system_drag_ended();
    view.push(ViewEvent::CursorChanged(CursorKind::Arrow));
    true
}

/// Abandon any drag (view closing, focus lost).
pub(crate) fn cancel(view: &Rc<ViewInner>) {
    let Some(session) = view.drag.borrow_mut().take() else { return };
    if let Some(host) = view.host() {
        host.drag_target_drag_leave();
        if session.from_page {
            host.drag_source_ended_at(0, 0, none());
            host.drag_source_system_drag_ended();
        }
    }
}

fn file_drag_data(paths: &[PathBuf]) -> Option<DragData> {
    let data = drag_data_create()?;
    for path in paths {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        data.add_file(
            Some(&CefString::from(path.to_string_lossy().as_ref())),
            Some(&CefString::from(name.as_str())),
        );
    }
    Some(data)
}

fn external_ops() -> DragOperationsMask {
    use cef::sys::cef_drag_operations_mask_t as M;
    op(M(M::DRAG_OPERATION_COPY.0 | M::DRAG_OPERATION_LINK.0 | M::DRAG_OPERATION_MOVE.0))
}

/// Files from outside are over the view.
pub(crate) fn external_over(view: &Rc<ViewInner>, paths: &[PathBuf], x: i32, y: i32, modifiers: u32) {
    let Some(host) = view.host() else { return };
    let event = MouseEvent { x, y, modifiers };
    let entering = view.drag.borrow().is_none();
    if entering {
        let Some(mut data) = file_drag_data(paths) else { return };
        *view.drag.borrow_mut() = Some(DragSession {
            allowed: external_ops(),
            current: none(),
            from_page: false,
        });
        host.drag_target_drag_enter(Some(&mut data), Some(&event), external_ops());
    }
    host.drag_target_drag_over(Some(&event), external_ops());
}

pub(crate) fn external_leave(view: &Rc<ViewInner>) {
    let external = view.drag.borrow().as_ref().is_some_and(|s| !s.from_page);
    if external {
        cancel(view);
    }
}

pub(crate) fn external_drop(view: &Rc<ViewInner>, paths: &[PathBuf], x: i32, y: i32, modifiers: u32) {
    external_over(view, paths, x, y, modifiers);
    let session = view.drag.borrow_mut().take();
    if let (Some(_), Some(host)) = (session, view.host()) {
        host.drag_target_drop(Some(&MouseEvent { x, y, modifiers }));
    }
}
