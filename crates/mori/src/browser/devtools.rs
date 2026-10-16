//! Chromium DevTools docked beside the page it inspects.
//!
//! DevTools is not a tab: it belongs to the inspected tab, shows only while
//! that tab is on screen (one pane at a time, the active split pane's
//! first), and is never saved to the session. F12 / ⌘⌥I
//! toggles it; Inspect opens it (or reuses it) on the clicked element. The
//! frontend runs in its own offscreen view, connected to the page through
//! [`DevToolsBridge`].

use gpui_kit::*;
use mori_core::workspace::TabId;
use mori_engine::{ViewEvent, WebView};

use super::Browser;
use crate::{devtools_bridge::DevToolsBridge, engine_hub::EngineHub, web_surface::WebSurface};

/// Share of the web column DevTools takes.
const PANE_FRACTION: f32 = 0.42;

pub struct DevToolsPane {
    pub surface: Entity<WebSurface>,
    pub view: WebView,
    /// The bridge session connecting this frontend to the page.
    token: String,
}

impl Browser {
    /// Open DevTools for the active page, or close it if it is open. With
    /// the keyboard in a DevTools pane, close that pane.
    pub fn toggle_devtools(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let focused = self
            .devtools
            .iter()
            .find(|(_, p)| p.surface.read(cx).focus_handle().is_focused(window))
            .map(|(t, _)| *t);
        let Some(id) = focused.or_else(|| self.active_tab()) else { return };
        if self.devtools.contains_key(&id) {
            self.drop_devtools(id, cx);
            self.focus_selected_page(window, cx);
        } else {
            self.open_devtools(id, None, window, cx);
        }
    }

    /// Inspect the element at `(x, y)` of `tab`'s page, opening DevTools if
    /// needed.
    pub fn inspect_element(&mut self, tab: TabId, x: i32, y: i32, window: &mut Window, cx: &mut Context<Self>) {
        // The menu reports view pixels; the protocol wants CSS pixels,
        // which differ by the page zoom.
        let factor = self
            .live
            .get(&tab)
            .map(|l| mori_core::zoom::factor_for_level(l.view.zoom_level()))
            .filter(|f| *f > 0.0)
            .unwrap_or(1.0);
        let (x, y) = ((x as f64 / factor).round() as i32, (y as f64 / factor).round() as i32);
        // Inspecting the right split pane makes it the active one, so its
        // DevTools is the pane shown.
        if self.split == Some(tab) && !self.split_focused {
            self.split_focused = true;
            cx.notify();
        }
        match self.devtools.get(&tab) {
            Some(pane) => {
                let token = pane.token.clone();
                DevToolsBridge::inspect(cx, &token, x, y);
            }
            None => self.open_devtools(tab, Some((x, y)), window, cx),
        }
    }

    fn open_devtools(&mut self, tab: TabId, inspect_at: Option<(i32, i32)>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(inspected) = self.live.get(&tab).map(|l| l.view.clone()) else { return };
        let Some(url) = DevToolsBridge::open(cx, inspected.id()) else {
            self.toast("Couldn't start Developer Tools", super::ToastStyle::Error, None, cx);
            return;
        };
        let Some(token) = DevToolsBridge::token_of(&url) else { return };
        if let Some((x, y)) = inspect_at {
            DevToolsBridge::inspect(cx, &token, x, y);
        }
        let (w, h) = inspected.size();
        let width = (w as f32 * PANE_FRACTION / (1. - PANE_FRACTION)) as i32;
        let view = WebView::new(&url, (width, h), window.scale_factor());
        let surface = cx.new(|cx| WebSurface::new(view.clone(), cx));
        let weak = cx.entity().downgrade();
        let handle = self.window_handle;
        EngineHub::register(cx, &surface, move |_, events, cx| {
            let weak = weak.clone();
            handle
                .update(cx, |_, window, cx| {
                    weak.update(cx, |this, cx| this.on_devtools_events(tab, events, window, cx)).ok();
                })
                .ok();
        });
        let focus = surface.read(cx).focus_handle().clone();
        window.focus(&focus, cx);
        self.devtools.insert(tab, DevToolsPane { surface, view, token });
        self.sync_view_visibility(cx);
        cx.notify();
    }

    fn on_devtools_events(&mut self, tab: TabId, events: Vec<ViewEvent>, window: &mut Window, cx: &mut Context<Self>) {
        for event in events {
            match event {
                ViewEvent::CloseRequested | ViewEvent::Closed | ViewEvent::RenderProcessGone => {
                    self.drop_devtools(tab, cx);
                    if self.selected() == Some(tab) {
                        self.focus_selected_page(window, cx);
                    }
                }
                // Links from DevTools (sources, docs) open as tabs.
                ViewEvent::NewView { view, url, .. } => {
                    view.close();
                    self.open_url(&url, true, window, cx);
                }
                ViewEvent::FileDialog(request) => {
                    if let Some(view) = self.devtools.get(&tab).map(|p| p.view.clone()) {
                        self.open_file_dialog(view, request, window, cx);
                    }
                }
                // Browser shortcuts DevTools did not use (F12, Ctrl+Shift+I,
                // Ctrl+R…) run as from any page.
                ViewEvent::UnhandledKey(key) => {
                    if let Some(surface) = self.devtools.get(&tab).map(|p| p.surface.clone()) {
                        surface.update(cx, |s, cx| s.run_unhandled_shortcut(key, window, cx));
                    }
                }
                _ => {}
            }
        }
    }

    /// Close `tab`'s DevTools pane, if any.
    pub(super) fn drop_devtools(&mut self, tab: TabId, cx: &mut Context<Self>) {
        if let Some(pane) = self.devtools.remove(&tab) {
            EngineHub::unregister(cx, pane.view.id());
            DevToolsBridge::close(cx, &pane.token);
            pane.view.close();
            cx.notify();
        }
    }

    /// The tab whose DevTools pane is on screen: the active pane's if it
    /// has one, else the other visible pane's.
    pub(super) fn shown_devtools_tab(&self) -> Option<TabId> {
        if self.settings_open || self.selected_live().is_some_and(|l| l.fullscreen) {
            return None;
        }
        let split = self.split.filter(|s| self.live.contains_key(s));
        self.active_tab()
            .into_iter()
            .chain(self.selected())
            .chain(split)
            .find(|t| self.devtools.contains_key(t))
    }

    /// The on-screen tab's DevTools, docked to the right of the pages.
    pub fn render_devtools_card(&self, cx: &App) -> Option<AnyElement> {
        use crate::theme::{ActiveTokens, metrics};
        let pane = self.devtools.get(&self.shown_devtools_tab()?)?;
        let t = cx.tokens();
        Some(
            div()
                .id("devtools-card")
                .flex_none()
                .ml(metrics::CARD_INSET)
                .w(relative(PANE_FRACTION))
                .min_w(px(320.))
                .h_full()
                .overflow_hidden()
                .bg(t.card)
                .rounded(metrics::RADIUS)
                .border_1()
                .border_color(t.border.opacity(0.7))
                .shadow(t.card_shadow())
                .child(pane.surface.clone())
                .into_any_element(),
        )
    }
}
