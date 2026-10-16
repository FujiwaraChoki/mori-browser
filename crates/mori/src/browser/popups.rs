//! `window.open` popups and pages that close themselves.
//!
//! A popup that asks for a size (`window.open(url, name, "width=480,...")`:
//! sign-in, payment and share dialogs) floats over its opener at that size
//! instead of replacing the page with a full tab, and `window.close()` from
//! it, or from a tab a page opened, returns to the opener.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use mori_core::workspace::{TabId, TabRecord};
use mori_engine::{Disposition, PopupFeatures, ViewEvent, WebView};

use super::Browser;
use crate::{engine_hub::EngineHub, web_surface::WebSurface};

/// Height of the popup's title strip.
const TITLE_BAR: f32 = 38.;

pub struct PopupWindow {
    pub surface: Entity<WebSurface>,
    pub view: WebView,
    pub opener: TabId,
    pub title: String,
    pub url: String,
    /// Page size the opener asked for, logical pixels.
    pub size: (f32, f32),
    _surface_events: Subscription,
}

/// Whether a new page should open as a floating popup rather than a tab.
pub(super) fn wants_window(disposition: Disposition, features: PopupFeatures) -> bool {
    matches!(disposition, Disposition::Popup | Disposition::Window)
        && features.width.is_some()
        && features.height.is_some()
}

impl Browser {
    pub(super) fn open_popup_window(
        &mut self,
        opener: TabId,
        view: WebView,
        url: String,
        features: PopupFeatures,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // One popup at a time: a new one replaces the old (as a page
        // reusing its named window would).
        self.close_popup_window(window, cx);
        let viewport = window.viewport_size();
        let max_w = (f32::from(viewport.width) - 48.).max(200.);
        let max_h = (f32::from(viewport.height) - 48. - TITLE_BAR).max(150.);
        let w = (features.width.unwrap_or(500) as f32).clamp(200., max_w);
        let h = (features.height.unwrap_or(600) as f32).clamp(150., max_h);
        view.resize(w as i32, h as i32, window.scale_factor());
        view.set_hidden(false);
        let surface = cx.new(|cx| WebSurface::new(view.clone(), cx));
        let surface_events = self.watch_surface(&surface, cx);
        let weak = cx.entity().downgrade();
        let handle = self.window_handle;
        EngineHub::register(cx, &surface, move |_, events, cx| {
            let weak = weak.clone();
            handle
                .update(cx, |_, window, cx| {
                    weak.update(cx, |this, cx| this.on_popup_events(events, window, cx)).ok();
                })
                .ok();
        });
        let focus = surface.read(cx).focus_handle().clone();
        window.focus(&focus, cx);
        self.popup = Some(PopupWindow {
            surface,
            view,
            opener,
            title: String::new(),
            url,
            size: (w, h),
            _surface_events: surface_events,
        });
        cx.notify();
    }

    fn on_popup_events(&mut self, events: Vec<ViewEvent>, window: &mut Window, cx: &mut Context<Self>) {
        for event in events {
            let Some(popup) = self.popup.as_mut() else { return };
            match event {
                ViewEvent::TitleChanged(title) => popup.title = title,
                ViewEvent::AddressChanged(url) => popup.url = url,
                ViewEvent::LoadEnd { .. } => {
                    popup.view.execute_js(mori_core::scripts::CLIPBOARD_BRIDGE_SCRIPT);
                }
                ViewEvent::CloseRequested | ViewEvent::Closed => self.close_popup_window(window, cx),
                ViewEvent::NewView { view, url, .. } => {
                    view.close();
                    self.open_url(&url, true, window, cx);
                }
                ViewEvent::FileDialog(request) => {
                    let view = popup.view.clone();
                    self.open_file_dialog(view, request, window, cx);
                }
                // The popup has no tab to anchor a prompt to; answer like a
                // dismissed dialog so the page does not hang.
                ViewEvent::JsDialog(dialog) => popup.view.resolve_dialog(dialog.id, false, None),
                ViewEvent::PermissionPrompt(request) => popup.view.resolve_permission(request.id, false),
                _ => {}
            }
        }
        cx.notify();
    }

    pub fn close_popup_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(popup) = self.popup.take() else { return };
        EngineHub::unregister(cx, popup.view.id());
        popup.view.close();
        if self.selected() != Some(popup.opener) && self.ws.tab(popup.opener).is_some() {
            self.select_tab(popup.opener, window, cx);
        } else {
            self.focus_selected_page(window, cx);
        }
        cx.notify();
    }

    /// The popup floats over its opener: it goes when the opener's page
    /// does (tab closed or put to sleep).
    pub(super) fn drop_popup_of(&mut self, opener: TabId, cx: &mut Context<Self>) {
        if self.popup.as_ref().is_some_and(|p| p.opener == opener)
            && let Some(popup) = self.popup.take()
        {
            EngineHub::unregister(cx, popup.view.id());
            popup.view.close();
            cx.notify();
        }
    }

    /// Move the popup's page into a tab next to its opener.
    pub fn popup_to_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(popup) = self.popup.take() else { return };
        EngineHub::unregister(cx, popup.view.id());
        let mut record = TabRecord::new(popup.view.url());
        record.title = popup.title;
        let id = self.ws.add_child_tab(record, popup.opener, None);
        self.adopt_view(id, popup.view, window, cx);
        if let Some(live) = self.live.get_mut(&id) {
            live.opener = Some(popup.opener);
        }
        self.select_tab(id, window, cx);
    }

    /// Close a tab without the "Closed … Undo" toast or a closed-tab entry
    /// (it closed itself, or only ever held a download), selecting `then`.
    pub(super) fn close_tab_quietly(
        &mut self,
        id: TabId,
        then: Option<TabId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let was_selected = self.selected() == Some(id);
        if self.take_tab(id, false, cx).is_none() {
            return;
        }
        if was_selected
            && !self.ws.active().ordered().is_empty()
            && let Some(next) = then.filter(|t| self.ws.tab(*t).is_some())
        {
            self.select_tab(next, window, cx);
            self.schedule_save(cx);
            cx.notify();
            return;
        }
        self.after_tabs_closed(was_selected, window, cx);
    }

    pub fn render_popup_window(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        use crate::{
            theme::{ActiveTokens, metrics},
            ui::components::{favicon, h_flex, icon_button, text_button, v_flex},
        };
        let popup = self.popup.as_ref()?;
        let t = cx.tokens().clone();
        let host = mori_core::util::host_of(&popup.url);
        let fav = host.as_deref().and_then(|h| self.favicons.get(h));
        let title = if popup.title.is_empty() { popup.url.clone() } else { popup.title.clone() };
        let secure = popup.url.starts_with("https://");
        let host_label = host.unwrap_or_else(|| popup.url.clone());
        let (w, h) = popup.size;
        let surface = popup.surface.clone();
        Some(
            div()
                .absolute()
                .inset_0()
                .occlude()
                .bg(black().opacity(0.28))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    v_flex()
                        .w(px(w + 2.))
                        .h(px(h + TITLE_BAR + 2.))
                        .rounded(metrics::RADIUS)
                        .overflow_hidden()
                        .bg(t.card)
                        .border_1()
                        .border_color(t.border.opacity(0.7))
                        .shadow(t.overlay_shadow())
                        .child(
                            h_flex()
                                .h(px(TITLE_BAR))
                                .flex_none()
                                .px_2()
                                .gap_2()
                                .child(favicon(fav, px(14.), false, cx))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_size(metrics::TEXT_LABEL)
                                        .child(title),
                                )
                                .child(
                                    h_flex()
                                        .gap_1()
                                        .text_size(metrics::TEXT_SMALL)
                                        .text_color(t.muted_foreground)
                                        .when(secure, |s| {
                                            s.child(crate::ui::components::icon("lock", px(11.), t.muted_foreground))
                                        })
                                        .child(host_label),
                                )
                                .child(text_button("popup-tab", "Open as Tab", false, cx).on_click(
                                    cx.listener(|this, _, window, cx| this.popup_to_tab(window, cx)),
                                ))
                                .child(icon_button("popup-close", "x", px(26.), true, cx).on_click(
                                    cx.listener(|this, _, window, cx| this.close_popup_window(window, cx)),
                                )),
                        )
                        .child(div().flex_1().min_h_0().child(surface)),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::wants_window;
    use mori_engine::{Disposition, PopupFeatures};

    fn sized(w: Option<i32>, h: Option<i32>) -> PopupFeatures {
        PopupFeatures { width: w, height: h }
    }

    #[test]
    fn only_sized_popups_float() {
        assert!(wants_window(Disposition::Popup, sized(Some(400), Some(300))));
        assert!(!wants_window(Disposition::Popup, sized(None, None)));
        assert!(!wants_window(Disposition::Popup, sized(Some(400), None)));
        assert!(!wants_window(Disposition::ForegroundTab, sized(Some(400), Some(300))));
    }
}
