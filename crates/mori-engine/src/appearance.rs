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

use cef::{CefString, ImplBrowserHost, ImplFrame};

use crate::view::ViewInner;

const UNSET: u8 = 0;
const LIGHT: u8 = 1;
const DARK: u8 = 2;

/// The scheme pages should see; new pages pick it up when created.
static WANTED: AtomicU8 = AtomicU8::new(UNSET);

/// Installed into every document as it starts loading. Page styles
/// still win: they come later in the cascade, and `scrollbar-width` /
/// `scrollbar-color` replace `::-webkit-scrollbar` styling entirely.
/// The thumb shows while the page scrolls or the pointer is over a
/// scrollbar, and fades out about a second later (GTK/Zen behaviour).
const SCROLLBAR_SCRIPT: &str = r#"(() => {
  if (window.__moriScrollbars) return;
  window.__moriScrollbars = true;
  const css = `
    ::-webkit-scrollbar { width: 10px; height: 10px; background: transparent; }
    ::-webkit-scrollbar-button, ::-webkit-scrollbar-corner { display: none; background: transparent; }
    ::-webkit-scrollbar-track { background: transparent; }
    ::-webkit-scrollbar-thumb {
      background-color: transparent; border-radius: 5px;
      border: 2px solid transparent; background-clip: content-box;
    }
    :root.__mori-scrolling ::-webkit-scrollbar-thumb, ::-webkit-scrollbar-thumb:hover {
      background-color: rgba(128, 128, 128, 0.55);
    }
    ::-webkit-scrollbar-thumb:active { background-color: rgba(128, 128, 128, 0.75); }
  `;
  let timer = 0;
  const scrolling = () => {
    const root = document.documentElement;
    if (!root) return;
    root.classList.add('__mori-scrolling');
    clearTimeout(timer);
    timer = setTimeout(() => root.classList.remove('__mori-scrolling'), 1000);
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

/// Fire-and-forget DevTools call. Ids come from the shared token counter so
/// they never collide with a pending `Runtime.evaluate` reply.
fn send(view: &Rc<ViewInner>, method: &str, params: serde_json::Value) {
    let Some(host) = view.host() else { return };
    let id = (crate::view::next_token() % i32::MAX as u64) as i32;
    let message = serde_json::json!({ "id": id, "method": method, "params": params });
    let bytes = message.to_string().into_bytes();
    if host.send_dev_tools_message(Some(&bytes)) != 1 {
        log::debug!("view {:?}: {method} rejected", view.id);
    }
}
