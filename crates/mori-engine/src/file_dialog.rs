//! File choosers (`<input type=file>`, `showOpenFilePicker`, ...).
//!
//! Offscreen browsers have no native window to parent a dialog to, so the
//! request goes to the chrome as [`ViewEvent::FileDialog`]; it shows the
//! system picker attached to the Mori window and answers through
//! [`crate::WebView::resolve_file_dialog`].

use std::rc::Weak;

use cef::*;

use crate::view::{FileDialogId, FileDialogMode, FileDialogRequest, ViewEvent, ViewInner, next_token};

pub(crate) fn build_handler(view: Weak<ViewInner>) -> DialogHandler {
    DialogHandlerImpl::new(view)
}

fn mode_of(mode: cef::FileDialogMode) -> FileDialogMode {
    if mode == cef::FileDialogMode::OPEN_MULTIPLE {
        FileDialogMode::OpenMultiple
    } else if mode == cef::FileDialogMode::OPEN_FOLDER {
        FileDialogMode::OpenFolder
    } else if mode == cef::FileDialogMode::SAVE {
        FileDialogMode::Save
    } else {
        FileDialogMode::Open
    }
}

wrap_dialog_handler! {
    pub(crate) struct DialogHandlerImpl {
        view: Weak<ViewInner>,
    }

    impl DialogHandler {
        fn on_file_dialog(
            &self,
            _browser: Option<&mut Browser>,
            mode: cef::FileDialogMode,
            title: Option<&CefString>,
            default_file_path: Option<&CefString>,
            accept_filters: Option<&mut CefStringList>,
            _accept_extensions: Option<&mut CefStringList>,
            _accept_descriptions: Option<&mut CefStringList>,
            callback: Option<&mut FileDialogCallback>,
        ) -> ::std::os::raw::c_int {
            let Some(callback) = callback else { return 0 };
            let Some(view) = self.view.upgrade().filter(|v| !v.closing.get()) else {
                callback.cancel();
                return 1;
            };
            let accept: Vec<String> = accept_filters
                // Take the borrowed list: cloning copies an opaque struct and reads empty.
                .map(|list| std::mem::take(list).into_iter().collect())
                .unwrap_or_default();
            let id = FileDialogId(next_token());
            view.file_dialogs.borrow_mut().insert(id, callback.clone());
            view.push(ViewEvent::FileDialog(FileDialogRequest {
                id,
                mode: mode_of(mode),
                title: title.map(|t| t.to_string()).unwrap_or_default(),
                default_path: default_file_path.map(|p| p.to_string()).unwrap_or_default(),
                accept,
            }));
            1
        }
    }
}
