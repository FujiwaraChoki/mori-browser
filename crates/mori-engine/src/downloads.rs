//! Downloads: saved straight into the downloads directory, reported to the
//! chrome as [`DownloadUpdate`]s, and controllable by id.

use std::{
    cell::RefCell,
    collections::HashMap,
    path::{Path, PathBuf},
    rc::Weak,
};

use cef::*;

use crate::view::{ViewEvent, ViewInner};

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
}

thread_local! {
    static DOWNLOADS_DIR: RefCell<PathBuf> = RefCell::new(PathBuf::from("."));
    static CONTROLS: RefCell<HashMap<DownloadId, DownloadItemCallback>> = RefCell::new(HashMap::new());
}

pub(crate) fn set_downloads_dir(dir: PathBuf) {
    DOWNLOADS_DIR.with(|d| *d.borrow_mut() = dir);
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

fn update_from(item: &DownloadItem, path_hint: Option<PathBuf>) -> DownloadUpdate {
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
            if let Some(view) = self.view.upgrade() {
                view.push(ViewEvent::Download(update_from(item, Some(path))));
            }
            1
        }

        fn on_download_updated(
            &self,
            _browser: Option<&mut Browser>,
            download_item: Option<&mut DownloadItem>,
            callback: Option<&mut DownloadItemCallback>,
        ) {
            let Some(item) = download_item else { return };
            let update = update_from(item, None);
            let id = update.id;
            match update.state {
                DownloadState::InProgress | DownloadState::Paused => {
                    if let Some(cb) = callback {
                        CONTROLS.with(|c| c.borrow_mut().insert(id, cb.clone()));
                    }
                }
                _ => {
                    CONTROLS.with(|c| c.borrow_mut().remove(&id));
                }
            }
            // A download whose path is still empty has not been accepted yet.
            if update.path.as_os_str().is_empty() {
                return;
            }
            if let Some(view) = self.view.upgrade() {
                view.push(ViewEvent::Download(update));
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
}
