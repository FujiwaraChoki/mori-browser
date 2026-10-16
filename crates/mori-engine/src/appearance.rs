//! How pages look inside Mori's chrome: light/dark mode and scrollbars.
//!
//! Offscreen pages on the headless ozone platform never see the desktop's
//! appearance, and Chromium's own colour-scheme setting only reaches
//! Chrome-style browsers, so `prefers-color-scheme` would always report
//! light. Mori emulates the media feature on every page through the DevTools
//! protocol instead: dark-aware sites (and their favicons) follow the chrome,
//! and switch live when it changes.
//!
//! Likewise Chromium only uses overlay scrollbars on Linux when GTK says so,
//! which it cannot under headless ozone; pages get a thin, auto-hiding
//! scrollbar style instead of the classic 15px bar with arrow buttons.

use std::{
    rc::Rc,
    sync::atomic::{AtomicU8, Ordering},
};

use cef::{CefString, ImplFrame};

use crate::view::ViewInner;

const UNSET: u8 = 0;
const LIGHT: u8 = 1;
const DARK: u8 = 2;

/// The scheme pages should see; new pages pick it up when created.
static WANTED: AtomicU8 = AtomicU8::new(UNSET);

/// Installed into every document as it starts loading. Page styles
/// still win: they come later in the cascade, and `scrollbar-width` /
/// `scrollbar-color` replace `::-webkit-scrollbar` styling entirely.
/// A thumb shows while its scroller scrolls or the pointer is over it, and
/// fades out about a second later (GTK/Zen behaviour). Only the element
/// that scrolled is marked, and the selector has no descendant combinator,
/// so toggling the mark restyles that one element rather than the whole
/// document (a full style recalc at every scroll start would jank large
/// pages).
const SCROLLBAR_SCRIPT: &str = r#"(() => {
  if (window.__moriScrollbars) return;
  window.__moriScrollbars = true;
  const mark = '__mori-scrolling';
  const css = `
    ::-webkit-scrollbar { width: 10px; height: 10px; background: transparent; }
    ::-webkit-scrollbar-button, ::-webkit-scrollbar-corner { display: none; background: transparent; }
    ::-webkit-scrollbar-track { background: transparent; }
    ::-webkit-scrollbar-thumb {
      background-color: transparent; border-radius: 5px;
      border: 2px solid transparent; background-clip: content-box;
    }
    .${mark}::-webkit-scrollbar-thumb, ::-webkit-scrollbar-thumb:hover {
      background-color: rgba(128, 128, 128, 0.55);
    }
    ::-webkit-scrollbar-thumb:active { background-color: rgba(128, 128, 128, 0.75); }
  `;
  const timers = new WeakMap();
  const show = (el) => {
    el.classList.add(mark);
    clearTimeout(timers.get(el));
    timers.set(el, setTimeout(() => el.classList.remove(mark), 1000));
  };
  const scrolling = (event) => {
    const target = event.target;
    if (target instanceof Element) {
      show(target);
      return;
    }
    // The page itself scrolled. Blink only repaints the viewport's
    // scrollbar for a change on <body>; mark <html> too for documents
    // without one.
    for (const el of [document.documentElement, document.body]) {
      if (el) show(el);
    }
  };
  const install = () => {
    const style = document.createElement('style');
    style.textContent = css;
    (document.head || document.documentElement).prepend(style);
  };
  if (document.documentElement) {
    install();
  } else {
    new MutationObserver((_, observer) => {
      if (document.documentElement) { observer.disconnect(); install(); }
    }).observe(document, { childList: true });
  }
  addEventListener('scroll', scrolling, { capture: true, passive: true });
})();"#;

/// Make web content render light or dark (`prefers-color-scheme`), now and
/// for every page created later. Must be called on the browser process main
/// thread.
pub fn set_color_scheme(dark: bool) {
    let wanted = if dark { DARK } else { LIGHT };
    if WANTED.swap(wanted, Ordering::SeqCst) == wanted {
        return;
    }
    for view in crate::view::all_views() {
        send_color_scheme(&view);
    }
}

/// Send the current colour scheme to one page again, e.g. after a DevTools
/// frontend detached and reset the page's media emulation.
pub fn reapply_color_scheme(view: &crate::WebView) {
    send_color_scheme(&view.0);
}

/// Give a page the current colour scheme once its browser exists; the
/// emulation holds across navigations.
pub(crate) fn attach(view: &Rc<ViewInner>) {
    send_color_scheme(view);
}

/// Install the scrollbar style into a frame's new document. Called when a
/// navigation commits, for every frame (cross-origin iframes included); the
/// script is idempotent per document.
pub(crate) fn on_load_start(frame: &cef::Frame) {
    frame.execute_java_script(Some(&CefString::from(SCROLLBAR_SCRIPT)), None, 0);
}

fn send_color_scheme(view: &Rc<ViewInner>) {
    let scheme = match WANTED.load(Ordering::SeqCst) {
        DARK => "dark",
        LIGHT => "light",
        _ => return,
    };
    send(
        view,
        "Emulation.setEmulatedMedia",
        serde_json::json!({ "features": [{ "name": "prefers-color-scheme", "value": scheme }] }),
    );
}

/// Fire-and-forget DevTools call, through Mori's own id range so the reply
/// is consumed here and never reaches an attached DevTools frontend.
fn send(view: &Rc<ViewInner>, method: &str, params: serde_json::Value) {
    let id = view.id;
    let method_name = method.to_string();
    crate::devtools::call(
        view,
        method,
        params,
        Box::new(move |result| {
            if let Err(e) = result {
                log::debug!("view {id:?}: {method_name} failed: {e}");
            }
        }),
    );
}
