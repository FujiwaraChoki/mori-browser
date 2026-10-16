//! Downloads: saved straight into the downloads directory, reported to the
//! chrome as [`DownloadUpdate`]s through one global queue (see
//! [`take_download_updates`]), and controllable by id.
//!
//! Updates do not travel with the originating view: its tab may close,
//! sleep or be a Peek while the download runs. That view is also parked
//! until its downloads settle (see `WebView::close`), because Chromium
//! cancels the downloads of a destroyed browser.

use std::{
    cell::RefCell,
    collections::HashMap,
    path::{Path, PathBuf},
    rc::Weak,
};

use cef::*;

use crate::view::{ViewId, ViewInner, note_download};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DownloadId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DownloadState {
    InProgress,
    Paused,
    Complete,
    Canceled,
    Interrupted,
}

impl DownloadState {
    /// Running or paused: more updates will come.
    pub fn is_active(self) -> bool {
        matches!(self, DownloadState::InProgress | DownloadState::Paused)
    }
}

#[derive(Clone, Debug)]
pub struct DownloadUpdate {
    pub id: DownloadId,
    pub url: String,
    pub filename: String,
    pub path: PathBuf,
    pub received: i64,
    pub total: i64,
    /// 0-100, or -1 when the size is unknown.
    pub percent: i32,
    /// Bytes per second.
    pub speed: i64,
    pub state: DownloadState,
    /// The view that started the download.
    pub source: Option<ViewId>,
}

thread_local! {
    static DOWNLOADS_DIR: RefCell<PathBuf> = RefCell::new(PathBuf::from("."));
    static CONTROLS: RefCell<HashMap<DownloadId, DownloadItemCallback>> = RefCell::new(HashMap::new());
    static UPDATES: RefCell<Vec<DownloadUpdate>> = const { RefCell::new(Vec::new()) };
}

pub(crate) fn set_downloads_dir(dir: PathBuf) {
    DOWNLOADS_DIR.with(|d| *d.borrow_mut() = dir);
}

/// Download updates since the last call, oldest first.
pub fn take_download_updates() -> Vec<DownloadUpdate> {
    UPDATES.with(|u| std::mem::take(&mut *u.borrow_mut()))
}

fn publish(update: DownloadUpdate) {
    UPDATES.with(|u| u.borrow_mut().push(update));
}

/// Cancel a running download.
pub fn cancel_download(id: DownloadId) {
    if let Some(cb) = CONTROLS.with(|c| c.borrow().get(&id).cloned()) {
        cb.cancel();
    }
}

pub fn pause_download(id: DownloadId) {
    if let Some(cb) = CONTROLS.with(|c| c.borrow().get(&id).cloned()) {
        cb.pause();
    }
}

pub fn resume_download(id: DownloadId) {
    if let Some(cb) = CONTROLS.with(|c| c.borrow().get(&id).cloned()) {
        cb.resume();
    }
}

/// Whether this session can still pause, resume or cancel the download.
pub fn is_controllable(id: DownloadId) -> bool {
    CONTROLS.with(|c| c.borrow().contains_key(&id))
}

/// Pick `dir/name`, or `dir/name (n).ext` if that already exists.
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let name = sanitize(name);
    let candidate = dir.join(&name);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name.as_str(), ""),
    };
    (1..10_000)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|p| !p.exists())
        .unwrap_or(candidate)
}

fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c == '/' || c == '\0' { '_' } else { c })
        .collect();
    let trimmed = cleaned.trim().trim_start_matches('.');
    if trimmed.is_empty() {
        "download".to_string()
    } else {
        trimmed.to_string()
    }
}

fn update_from(item: &DownloadItem, path_hint: Option<PathBuf>, source: Option<ViewId>) -> DownloadUpdate {
    let s = |u: &CefStringUserfree| CefString::from(u).to_string();
    let path = path_hint.unwrap_or_else(|| PathBuf::from(s(&item.full_path())));
    let filename = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| s(&item.suggested_file_name()));
    let state = if item.is_complete() == 1 {
        DownloadState::Complete
    } else if item.is_canceled() == 1 {
        DownloadState::Canceled
    } else if item.is_interrupted() == 1 {
        DownloadState::Interrupted
    } else if item.is_paused() == 1 {
        DownloadState::Paused
    } else {
        DownloadState::InProgress
    };
    DownloadUpdate {
        id: DownloadId(item.id()),
        url: s(&item.url()),
        filename,
        path,
        received: item.received_bytes(),
        total: item.total_bytes(),
        percent: item.percent_complete(),
        speed: item.current_speed(),
        state,
        source,
    }
}

pub(crate) fn build_handler(view: Weak<ViewInner>) -> DownloadHandler {
    DownloadHandlerImpl::new(view)
}

wrap_download_handler! {
    pub(crate) struct DownloadHandlerImpl {
        view: Weak<ViewInner>,
    }

    impl DownloadHandler {
        fn can_download(
            &self,
            _browser: Option<&mut Browser>,
            _url: Option<&CefString>,
            _request_method: Option<&CefString>,
        ) -> ::std::os::raw::c_int {
            1
        }

        fn on_before_download(
            &self,
            _browser: Option<&mut Browser>,
            download_item: Option<&mut DownloadItem>,
            suggested_name: Option<&CefString>,
            callback: Option<&mut BeforeDownloadCallback>,
        ) -> ::std::os::raw::c_int {
            let (Some(item), Some(callback)) = (download_item, callback) else { return 0 };
            let name = suggested_name.map(|s| s.to_string()).unwrap_or_default();
            let dir = DOWNLOADS_DIR.with(|d| d.borrow().clone());
            let path = unique_path(&dir, &name);
            callback.cont(Some(&CefString::from(path.to_string_lossy().as_ref())), 0);
            let view = self.view.upgrade();
            if let Some(view) = &view {
                note_download(view, DownloadId(item.id()), true);
            }
            publish(update_from(item, Some(path), view.map(|v| v.id)));
            1
        }

        fn on_download_updated(
            &self,
            _browser: Option<&mut Browser>,
            download_item: Option<&mut DownloadItem>,
            callback: Option<&mut DownloadItemCallback>,
        ) {
            let Some(item) = download_item else { return };
            let view = self.view.upgrade();
            let update = update_from(item, None, view.as_ref().map(|v| v.id));
            let id = update.id;
            let active = update.state.is_active();
            if active {
                if let Some(cb) = callback {
                    CONTROLS.with(|c| c.borrow_mut().insert(id, cb.clone()));
                }
            } else {
                CONTROLS.with(|c| c.borrow_mut().remove(&id));
            }
            // A download whose path is still empty has not been accepted yet.
            if update.path.as_os_str().is_empty() {
                return;
            }
            publish(update);
            // Last: settling a parked view's final download closes it.
            if let Some(view) = view {
                note_download(&view, id, active);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_names() {
        assert_eq!(sanitize("../etc/passwd"), "_etc_passwd");
        assert_eq!(sanitize("   "), "download");
        assert_eq!(sanitize(".hidden"), "hidden");
    }

    #[test]
    fn unique_path_appends_counter() {
        let dir = std::env::temp_dir().join(format!("mori-dl-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "x").unwrap();
        assert_eq!(unique_path(&dir, "a.txt"), dir.join("a (1).txt"));
        assert_eq!(unique_path(&dir, "b.txt"), dir.join("b.txt"));
    }

    #[test]
    fn active_states() {
        assert!(DownloadState::InProgress.is_active());
        assert!(DownloadState::Paused.is_active());
        assert!(!DownloadState::Complete.is_active());
        assert!(!DownloadState::Interrupted.is_active());
    }
}
