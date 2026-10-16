//! Downloads: progress from the engine's global queue into the persisted
//! list, small toasts instead of a panel that pops over the page, and the
//! pause / resume / cancel / retry controls.

use gpui_kit::*;
use mori_core::downloads::{DownloadChange, DownloadProgress, DownloadState};
use mori_engine::{DownloadId, DownloadUpdate, ViewId};
use uuid::Uuid;

use super::{Browser, ToastAction, ToastStyle};
use crate::engine_hub::EngineHub;

fn progress_of(update: &DownloadUpdate) -> DownloadProgress {
    use mori_engine::DownloadState as E;
    DownloadProgress {
        engine_id: update.id.0,
        url: update.url.clone(),
        filename: update.filename.clone(),
        path: update.path.clone(),
        received: update.received,
        total: update.total,
        percent: update.percent,
        speed: update.speed,
        state: match update.state {
            E::InProgress => DownloadState::InProgress,
            E::Paused => DownloadState::Paused,
            E::Complete => DownloadState::Complete,
            E::Canceled => DownloadState::Canceled,
            E::Interrupted => DownloadState::Interrupted,
        },
    }
}

impl Browser {
    /// Route the engine's download updates to this window.
    pub(super) fn watch_downloads(&mut self, cx: &mut Context<Self>) {
        let weak = cx.entity().downgrade();
        let handle = self.window_handle;
        EngineHub::on_downloads(cx, move |updates, cx| {
            let weak = weak.clone();
            handle
                .update(cx, |_, window, cx| {
                    weak.update(cx, |this, cx| this.on_download_updates(updates, window, cx)).ok();
                })
                .ok();
        });
    }

    fn on_download_updates(&mut self, updates: Vec<DownloadUpdate>, window: &mut Window, cx: &mut Context<Self>) {
        let mut save = false;
        for update in updates {
            // Remember which tab started it, so its page isn't slept mid-way.
            if let Some(tab) = update
                .source
                .and_then(|v| self.live.iter().find(|(_, l)| l.view.id() == v).map(|(t, _)| *t))
            {
                self.download_tabs.insert(update.id, tab);
            }
            let name = update.filename.clone();
            let path = update.path.clone();
            match self.downloads.apply(progress_of(&update), mori_core::paths::now()) {
                DownloadChange::Started => {
                    save = true;
                    self.toast_keyed(
                        "download",
                        format!("Downloading {name}"),
                        ToastStyle::Info,
                        Some(("Show".into(), ToastAction::OpenDownloads)),
                        cx,
                    );
                    if let Some(source) = update.source {
                        self.drop_download_only_tab(source, window, cx);
                    }
                }
                DownloadChange::Finished(DownloadState::Complete) => {
                    save = true;
                    self.toast_keyed(
                        "download",
                        format!("Downloaded {name}"),
                        ToastStyle::Success,
                        Some(("Show in Folder".into(), ToastAction::Reveal(path))),
                        cx,
                    );
                }
                DownloadChange::Finished(DownloadState::Interrupted) => {
                    save = true;
                    self.toast_keyed(
                        "download",
                        format!("Download of {name} failed"),
                        ToastStyle::Warning,
                        Some(("Show".into(), ToastAction::OpenDownloads)),
                        cx,
                    );
                }
                DownloadChange::Finished(_) => {
                    save = true;
                    self.dismiss_keyed_toast("download", cx);
                }
                DownloadChange::Progressed => {}
            }
        }
        if save {
            self.schedule_save(cx);
        }
        cx.notify();
    }

    /// A tab a page opened (target=_blank) whose first navigation turned
    /// into a download has nothing to show: drop it and go back to the
    /// page that opened it, as Chrome does. The download keeps running.
    fn drop_download_only_tab(&mut self, source: ViewId, window: &mut Window, cx: &mut Context<Self>) {
        let Some((tab, opener)) = self
            .live
            .iter()
            // Never a tab that already shows a document, even one still
            // loading (a page may start a download before it finishes).
            .find(|(_, l)| {
                l.view.id() == source && l.blank_popup && matches!(l.view.url().as_str(), "" | "about:blank")
            })
            .map(|(tab, l)| (*tab, l.opener))
        else {
            return;
        };
        self.close_tab_quietly(tab, opener, window, cx);
    }

    pub fn pause_download(&mut self, id: Uuid) {
        if let Some(engine) = self.engine_download(id) {
            mori_engine::pause_download(engine);
        }
    }

    pub fn resume_download(&mut self, id: Uuid) {
        if let Some(engine) = self.engine_download(id) {
            mori_engine::resume_download(engine);
        }
    }

    pub fn cancel_download(&mut self, id: Uuid) {
        if let Some(engine) = self.engine_download(id) {
            mori_engine::cancel_download(engine);
        }
    }

    /// Download a failed or canceled entry again, from the current page so
    /// cookies and referrer apply.
    pub fn retry_download(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        let Some(url) = self.downloads.iter().find(|e| e.id == id).map(|e| e.url.clone()) else {
            return;
        };
        self.downloads.remove(id);
        let view = self.selected_view().or_else(|| self.live.values().next().map(|l| l.view.clone()));
        match view {
            Some(view) => view.start_download(&url),
            None => self.open_url(&url, true, window, cx),
        }
        cx.notify();
    }

    pub fn remove_download(&mut self, id: Uuid, cx: &mut Context<Self>) {
        self.downloads.remove(id);
        self.schedule_save(cx);
        cx.notify();
    }

    pub fn clear_finished_downloads(&mut self, cx: &mut Context<Self>) {
        self.downloads.clear_finished();
        self.schedule_save(cx);
        cx.notify();
    }

    /// The engine id of a download this session can still control.
    fn engine_download(&self, id: Uuid) -> Option<DownloadId> {
        let engine = self.downloads.iter().find(|e| e.id == id)?.engine_id?;
        let engine = DownloadId(engine);
        mori_engine::is_controllable(engine).then_some(engine)
    }

    /// Show a toast that replaces the previous one with the same key
    /// instead of stacking (download progress, zoom level).
    pub fn toast_keyed(
        &mut self,
        key: &'static str,
        text: impl Into<SharedString>,
        style: ToastStyle,
        action: Option<(SharedString, ToastAction)>,
        cx: &mut Context<Self>,
    ) {
        if let Some(old) = self.keyed_toasts.remove(key) {
            self.toasts.retain(|t| t.id != old);
        }
        self.toast(text, style, action, cx);
        if let Some(toast) = self.toasts.last() {
            self.keyed_toasts.insert(key, toast.id);
        }
    }

    pub fn dismiss_keyed_toast(&mut self, key: &'static str, cx: &mut Context<Self>) {
        if let Some(id) = self.keyed_toasts.remove(key) {
            self.dismiss_toast(id, cx);
        }
    }
}
