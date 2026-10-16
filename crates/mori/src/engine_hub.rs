//! Bridges the CEF message pump into GPUI.
//!
//! A foreground task pumps Chromium, then routes each dirty view's paints to
//! its [`WebSurface`] and its events to whoever registered for that view.
//! Events of a view nobody has registered for yet (a popup whose opener's
//! `NewView` is still on its way) stay queued in the engine until it is
//! registered. Downloads are not tied to a view and go to one global
//! handler.

use std::{collections::HashMap, rc::Rc, time::Duration};

use gpui_kit::*;
use mori_engine::{DownloadUpdate, ViewEvent, ViewId, WebView};

use crate::web_surface::WebSurface;

type EventHandler = Rc<dyn Fn(ViewId, Vec<ViewEvent>, &mut App)>;

struct Route {
    surface: WeakEntity<WebSurface>,
    on_events: EventHandler,
}

type RelaunchHandler = Rc<dyn Fn(Vec<String>, &mut App)>;
type DownloadHandler = Rc<dyn Fn(Vec<DownloadUpdate>, &mut App)>;

#[derive(Default)]
pub struct EngineHub {
    routes: HashMap<ViewId, Route>,
    on_relaunch: Option<RelaunchHandler>,
    on_downloads: Option<DownloadHandler>,
}

impl Global for EngineHub {}

impl EngineHub {
    pub fn register(
        cx: &mut App,
        surface: &Entity<WebSurface>,
        on_events: impl Fn(ViewId, Vec<ViewEvent>, &mut App) + 'static,
    ) {
        let view = surface.read(cx).view().clone();
        cx.default_global::<EngineHub>().routes.insert(
            view.id(),
            Route {
                surface: surface.downgrade(),
                on_events: Rc::new(on_events),
            },
        );
        // Deliver whatever queued up before anyone listened.
        if view.has_events() {
            view.mark_dirty();
        }
    }

    /// Called with the URL arguments of each later `mori` launch.
    pub fn on_relaunch(cx: &mut App, handler: impl Fn(Vec<String>, &mut App) + 'static) {
        cx.default_global::<EngineHub>().on_relaunch = Some(Rc::new(handler));
    }

    /// Called with every batch of download progress, from any view.
    pub fn on_downloads(cx: &mut App, handler: impl Fn(Vec<DownloadUpdate>, &mut App) + 'static) {
        cx.default_global::<EngineHub>().on_downloads = Some(Rc::new(handler));
    }

    pub fn unregister(cx: &mut App, id: ViewId) {
        cx.default_global::<EngineHub>().routes.remove(&id);
    }

    /// Start pumping Chromium from the GPUI foreground executor.
    ///
    /// The loop sleeps until CEF's next scheduled deadline, and CEF's
    /// schedule callback wakes it early, so an idle browser costs almost
    /// nothing while input and paints are handled immediately.
    pub fn start(cx: &mut App) {
        use futures::{FutureExt as _, StreamExt as _};

        cx.default_global::<EngineHub>();
        let (wake_tx, mut wake_rx) = futures::channel::mpsc::unbounded::<()>();
        mori_engine::set_pump_waker(move || {
            let _ = wake_tx.unbounded_send(());
        });
        cx.spawn(async move |cx| {
            loop {
                let delay = mori_engine::next_pump_delay_ms(1, 33);
                let timer = cx.background_executor().timer(Duration::from_millis(delay));
                futures::select_biased! {
                    _ = wake_rx.next() => {}
                    _ = timer.fuse() => {}
                }
                while wake_rx.try_recv().is_ok() {}
                mori_engine::pump();
                let relaunches = mori_engine::take_relaunches();
                if !relaunches.is_empty() {
                    cx.update(|cx| {
                        if let Some(handler) = cx.global::<EngineHub>().on_relaunch.clone() {
                            for urls in relaunches {
                                handler(urls, cx);
                            }
                        }
                    });
                }
                let downloads = mori_engine::take_download_updates();
                if !downloads.is_empty() {
                    cx.update(|cx| {
                        if let Some(handler) = cx.global::<EngineHub>().on_downloads.clone() {
                            handler(downloads, cx);
                        }
                    });
                }
                let dirty = mori_engine::take_dirty_views();
                if dirty.is_empty() {
                    continue;
                }
                cx.update(|cx| Self::dispatch(dirty, cx));
            }
        })
        .detach();
    }

    fn dispatch(dirty: Vec<ViewId>, cx: &mut App) {
        for id in dirty {
            let Some(view) = WebView::by_id(id) else { continue };
            let Some((surface, handler)) = cx
                .global::<EngineHub>()
                .routes
                .get(&id)
                .map(|route| (route.surface.upgrade(), route.on_events.clone()))
            else {
                // Nobody listens: keep the events for `register`, unless the
                // view is on its way out.
                if view.is_closing() {
                    drop(view.take_events());
                }
                continue;
            };
            let events = view.take_events();
            if let Some(surface) = surface {
                surface.update(cx, |s, cx| {
                    s.sync_frame(cx);
                    for event in &events {
                        match event {
                            ViewEvent::PopupWidgetChanged => s.sync_popup(cx),
                            ViewEvent::CursorChanged(kind) => s.set_cursor(*kind, cx),
                            ViewEvent::ImeCompositionBounds(rects) => {
                                s.set_composition_bounds(rects)
                            }
                            _ => {}
                        }
                    }
                });
            }
            if !events.is_empty() {
                handler(id, events, cx);
            }
        }
    }
}
