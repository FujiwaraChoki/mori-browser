//! Window-wide pointer hooks: revealing and auto-hiding the hidden sidebar,
//! and giving the keyboard back to the page after a click on plain chrome.
//! They listen to every pointer event in the window, whatever element is
//! under the pointer, so an overlay or a web page can't swallow them.

use gpui_kit::*;

use crate::browser::Browser;

impl Browser {
    /// An invisible element that registers the window-wide listeners.
    pub fn render_pointer_hooks(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let weak = cx.entity().downgrade();
        canvas(
            |_, _, _| (),
            move |_, _, window, _| {
                let this = weak.clone();
                window.on_mouse_event(move |ev: &MouseMoveEvent, phase, window, cx| {
                    if phase == DispatchPhase::Capture {
                        let width = f32::from(window.viewport_size().width);
                        let x = f32::from(ev.position.x);
                        this.update(cx, |b, cx| b.track_sidebar_peek(Some(x), width, cx))
                            .ok();
                    }
                });
                let this = weak.clone();
                window.on_mouse_event(move |_: &MouseExitEvent, phase, window, cx| {
                    if phase == DispatchPhase::Capture {
                        let width = f32::from(window.viewport_size().width);
                        this.update(cx, |b, cx| b.track_sidebar_peek(None, width, cx)).ok();
                    }
                });
                let this = weak.clone();
                window.on_mouse_event(move |_: &MouseUpEvent, phase, window, cx| {
                    if phase != DispatchPhase::Bubble {
                        return;
                    }
                    // Check once every click handler has run.
                    let (this, handle) = (this.clone(), window.window_handle());
                    cx.defer(move |cx| {
                        handle
                            .update(cx, |_, window, cx| {
                                this.update(cx, |b, cx| b.after_click(window, cx)).ok();
                            })
                            .ok();
                    });
                });
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_0()
    }
}
