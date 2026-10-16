//! The CEF `Client` for each view: every Chromium callback Mori listens to.
//!
//! Handlers hold a `Weak` reference to their view so a closed view is freed
//! even while Chromium still holds the client.

// `wrap_client!` generates a constructor taking one argument per handler.
#![allow(clippy::too_many_arguments)]

use std::{
    cell::RefCell,
    rc::{Rc, Weak},
};

use cef::*;

use crate::view::{
    ContextMenuInfo, CursorKind, DialogId, DialogKind, Disposition, Frame, JsDialog,
    PendingPermission, PermissionId, PermissionKind, PermissionRequest, PopupFrame, ViewEvent,
    ViewInner, WebView, next_token,
};

thread_local! {
    /// Views Chromium is creating for us (popups, DevTools). Holds the strong
    /// reference until the browser exists and the opener has been told.
    static PENDING: RefCell<Vec<Rc<ViewInner>>> = const { RefCell::new(Vec::new()) };
}

/// How a pending view should be announced once its browser exists.
struct Announce {
    opener: Weak<ViewInner>,
    url: String,
    disposition: Disposition,
    user_gesture: bool,
}

thread_local! {
    static ANNOUNCE: RefCell<Vec<(crate::ViewId, Announce)>> = const { RefCell::new(Vec::new()) };
}

pub(crate) fn browser_settings() -> BrowserSettings {
    BrowserSettings {
        windowless_frame_rate: 60,
        background_color: 0xFFFFFFFF,
        ..Default::default()
    }
}

fn cef_str(s: Option<&CefString>) -> String {
    s.map(|s| s.to_string()).unwrap_or_default()
}

fn userfree(s: &CefStringUserfree) -> String {
    CefString::from(s).to_string()
}

fn non_empty(s: String) -> Option<String> {
    if s.is_empty() { None } else { Some(s) }
}

pub(crate) fn build_client(inner: &Rc<ViewInner>) -> Client {
    let weak = Rc::downgrade(inner);
    MoriClient::new(
        RenderHandlerImpl::new(weak.clone()),
        DisplayHandlerImpl::new(weak.clone()),
        LoadHandlerImpl::new(weak.clone()),
        LifeSpanHandlerImpl::new(weak.clone()),
        RequestHandlerImpl::new(weak.clone()),
        JsDialogHandlerImpl::new(weak.clone()),
        PermissionHandlerImpl::new(weak.clone()),
        ContextMenuHandlerImpl::new(weak.clone()),
        FindHandlerImpl::new(weak.clone()),
        crate::downloads::build_handler(weak),
    )
}

/// Create a view whose browser Chromium (not us) will create, e.g. a popup.
fn pending_view(opener: &Rc<ViewInner>, announce: Announce) -> (Rc<ViewInner>, Client) {
    let inner = ViewInner::new(opener.size.get(), opener.scale.get());
    let client = build_client(&inner);
    ANNOUNCE.with(|a| a.borrow_mut().push((inner.id, announce)));
    PENDING.with(|p| p.borrow_mut().push(inner.clone()));
    (inner, client)
}

pub(crate) fn show_devtools(inspected: &Rc<ViewInner>, inspect_at: Option<(i32, i32)>) {
    let Some(host) = inspected.host() else { return };
    let (_, mut client) = pending_view(
        inspected,
        Announce {
            opener: Rc::downgrade(inspected),
            url: "devtools://devtools".into(),
            disposition: Disposition::ForegroundTab,
            user_gesture: true,
        },
    );
    let window_info = WindowInfo {
        windowless_rendering_enabled: true as _,
        ..Default::default()
    };
    let settings = browser_settings();
    let point = inspect_at.map(|(x, y)| Point { x, y });
    host.show_dev_tools(
        Some(&window_info),
        Some(&mut client),
        Some(&settings),
        point.as_ref(),
    );
}

wrap_client! {
    pub(crate) struct MoriClient {
        render: RenderHandler,
        display: DisplayHandler,
        load: LoadHandler,
        life_span: LifeSpanHandler,
        request: RequestHandler,
        jsdialog: JsdialogHandler,
        permission: PermissionHandler,
        context_menu: ContextMenuHandler,
        find: FindHandler,
        download: DownloadHandler,
    }

    impl Client {
        fn render_handler(&self) -> Option<RenderHandler> { Some(self.render.clone()) }
        fn display_handler(&self) -> Option<DisplayHandler> { Some(self.display.clone()) }
        fn load_handler(&self) -> Option<LoadHandler> { Some(self.load.clone()) }
        fn life_span_handler(&self) -> Option<LifeSpanHandler> { Some(self.life_span.clone()) }
        fn request_handler(&self) -> Option<RequestHandler> { Some(self.request.clone()) }
        fn jsdialog_handler(&self) -> Option<JsdialogHandler> { Some(self.jsdialog.clone()) }
        fn permission_handler(&self) -> Option<PermissionHandler> { Some(self.permission.clone()) }
        fn context_menu_handler(&self) -> Option<ContextMenuHandler> { Some(self.context_menu.clone()) }
        fn find_handler(&self) -> Option<FindHandler> { Some(self.find.clone()) }
        fn download_handler(&self) -> Option<DownloadHandler> { Some(self.download.clone()) }
    }
}

// ---- Rendering ------------------------------------------------------------

wrap_render_handler! {
    pub(crate) struct RenderHandlerImpl {
        view: Weak<ViewInner>,
    }

    impl RenderHandler {
        fn view_rect(&self, _browser: Option<&mut Browser>, rect: Option<&mut Rect>) {
            let (Some(view), Some(rect)) = (self.view.upgrade(), rect) else { return };
            let (w, h) = view.size.get();
            *rect = Rect { x: 0, y: 0, width: w, height: h };
        }

        fn screen_info(
            &self,
            _browser: Option<&mut Browser>,
            screen_info: Option<&mut ScreenInfo>,
        ) -> ::std::os::raw::c_int {
            let (Some(view), Some(info)) = (self.view.upgrade(), screen_info) else {
                return 0;
            };
            let (w, h) = view.size.get();
            let (ox, oy) = view.screen_origin.get();
            let rect = Rect { x: ox, y: oy, width: w, height: h };
            info.device_scale_factor = view.scale.get();
            info.depth = 24;
            info.depth_per_component = 8;
            info.rect = rect.clone();
            info.available_rect = rect;
            1
        }

        fn screen_point(
            &self,
            _browser: Option<&mut Browser>,
            view_x: ::std::os::raw::c_int,
            view_y: ::std::os::raw::c_int,
            screen_x: Option<&mut ::std::os::raw::c_int>,
            screen_y: Option<&mut ::std::os::raw::c_int>,
        ) -> ::std::os::raw::c_int {
            let Some(view) = self.view.upgrade() else { return 0 };
            let (ox, oy) = view.screen_origin.get();
            if let Some(x) = screen_x { *x = ox + view_x; }
            if let Some(y) = screen_y { *y = oy + view_y; }
            1
        }

        fn on_popup_show(&self, _browser: Option<&mut Browser>, show: ::std::os::raw::c_int) {
            let Some(view) = self.view.upgrade() else { return };
            view.popup_visible.set(show == 1);
            if show != 1 {
                *view.popup.borrow_mut() = None;
            }
            view.push(ViewEvent::PopupWidgetChanged);
        }

        fn on_popup_size(&self, _browser: Option<&mut Browser>, rect: Option<&Rect>) {
            let (Some(view), Some(rect)) = (self.view.upgrade(), rect) else { return };
            let mut popup = view.popup.borrow_mut();
            let popup = popup.get_or_insert_with(PopupFrame::default);
            popup.x = rect.x;
            popup.y = rect.y;
            popup.width = rect.width;
            popup.height = rect.height;
        }

        fn on_paint(
            &self,
            _browser: Option<&mut Browser>,
            type_: PaintElementType,
            _dirty_rects: Option<&[Rect]>,
            buffer: *const u8,
            width: ::std::os::raw::c_int,
            height: ::std::os::raw::c_int,
        ) {
            let Some(view) = self.view.upgrade() else { return };
            if buffer.is_null() || width <= 0 || height <= 0 {
                return;
            }
            let len = width as usize * height as usize * 4;
            // SAFETY: CEF guarantees `buffer` holds width*height BGRA pixels
            // for the duration of this callback.
            let bytes = unsafe { std::slice::from_raw_parts(buffer, len) }.to_vec();
            let frame = Frame { width: width as u32, height: height as u32, bgra: bytes };
            if type_ == PaintElementType::POPUP {
                let mut popup = view.popup.borrow_mut();
                popup.get_or_insert_with(PopupFrame::default).frame = frame;
                drop(popup);
                view.push(ViewEvent::PopupWidgetChanged);
            } else {
                *view.frame.borrow_mut() = Some(frame);
                view.frame_generation.set(view.frame_generation.get() + 1);
                view.mark_dirty();
            }
        }

        fn on_ime_composition_range_changed(
            &self,
            _browser: Option<&mut Browser>,
            _selected_range: Option<&cef::Range>,
            character_bounds: Option<&[Rect]>,
        ) {
            let Some(view) = self.view.upgrade() else { return };
            let bounds = character_bounds
                .unwrap_or_default()
                .iter()
                .map(|r| (r.x, r.y, r.width, r.height))
                .collect();
            view.push(ViewEvent::ImeCompositionBounds(bounds));
        }

        fn on_text_selection_changed(
            &self,
            _browser: Option<&mut Browser>,
            selected_text: Option<&CefString>,
            _selected_range: Option<&cef::Range>,
        ) {
            let Some(view) = self.view.upgrade() else { return };
            view.push(ViewEvent::TextSelectionChanged(cef_str(selected_text)));
        }

        fn start_dragging(
            &self,
            browser: Option<&mut Browser>,
            _drag_data: Option<&mut DragData>,
            _allowed_ops: DragOperationsMask,
            _x: ::std::os::raw::c_int,
            _y: ::std::os::raw::c_int,
        ) -> ::std::os::raw::c_int {
            // Native drag sessions out of an offscreen view are not supported
            // yet; tell Chromium the drag ended so the page is not stuck.
            if let Some(host) = browser.and_then(|b| b.host()) {
                host.drag_source_system_drag_ended();
            }
            0
        }
    }
}

// ---- Display --------------------------------------------------------------

fn cursor_kind(t: CursorType) -> CursorKind {
    match t {
        CursorType::HAND => CursorKind::Pointer,
        CursorType::IBEAM | CursorType::VERTICALTEXT => CursorKind::Text,
        CursorType::CROSS | CursorType::CELL => CursorKind::Crosshair,
        CursorType::WAIT | CursorType::PROGRESS => CursorKind::Wait,
        CursorType::HELP => CursorKind::Help,
        CursorType::MOVE => CursorKind::Move,
        CursorType::NOTALLOWED | CursorType::NODROP => CursorKind::NotAllowed,
        CursorType::GRAB => CursorKind::Grab,
        CursorType::GRABBING => CursorKind::Grabbing,
        CursorType::EASTRESIZE | CursorType::WESTRESIZE | CursorType::EASTWESTRESIZE => {
            CursorKind::ResizeEw
        }
        CursorType::NORTHRESIZE | CursorType::SOUTHRESIZE | CursorType::NORTHSOUTHRESIZE => {
            CursorKind::ResizeNs
        }
        CursorType::NORTHEASTRESIZE
        | CursorType::SOUTHWESTRESIZE
        | CursorType::NORTHEASTSOUTHWESTRESIZE => CursorKind::ResizeNesw,
        CursorType::NORTHWESTRESIZE
        | CursorType::SOUTHEASTRESIZE
        | CursorType::NORTHWESTSOUTHEASTRESIZE => CursorKind::ResizeNwse,
        CursorType::COLUMNRESIZE => CursorKind::ResizeCol,
        CursorType::ROWRESIZE => CursorKind::ResizeRow,
        CursorType::NONE => CursorKind::None,
        _ => CursorKind::Arrow,
    }
}

wrap_display_handler! {
    pub(crate) struct DisplayHandlerImpl {
        view: Weak<ViewInner>,
    }

    impl DisplayHandler {
        fn on_address_change(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut cef::Frame>,
            url: Option<&CefString>,
        ) {
            let Some(view) = self.view.upgrade() else { return };
            if frame.map(|f| f.is_main() == 1).unwrap_or(true) {
                view.push(ViewEvent::AddressChanged(cef_str(url)));
            }
        }

        fn on_title_change(&self, _browser: Option<&mut Browser>, title: Option<&CefString>) {
            let Some(view) = self.view.upgrade() else { return };
            view.push(ViewEvent::TitleChanged(cef_str(title)));
        }

        fn on_favicon_urlchange(
            &self,
            browser: Option<&mut Browser>,
            icon_urls: Option<&mut CefStringList>,
        ) {
            let Some(view) = self.view.upgrade() else { return };
            let urls: Vec<String> = icon_urls
                // Take the borrowed list: cloning copies an opaque struct and reads empty.
                .map(|list| std::mem::take(list).into_iter().collect())
                .unwrap_or_default();
            let first = urls.into_iter().find(|u| !u.is_empty());
            log::debug!("favicon urls for view {:?}: {first:?}", view.id);
            view.push(ViewEvent::FaviconUrlChanged(first.clone()));
            if let (Some(url), Some(host)) = (first, browser.and_then(|b| b.host())) {
                let size = (32.0 * view.scale.get()).round() as u32;
                let mut cb = FaviconCallback::new(Rc::downgrade(&view));
                host.download_image(Some(&CefString::from(url.as_str())), 1, size, 0, Some(&mut cb));
            }
        }

        fn on_fullscreen_mode_change(
            &self,
            _browser: Option<&mut Browser>,
            fullscreen: ::std::os::raw::c_int,
        ) {
            let Some(view) = self.view.upgrade() else { return };
            view.push(ViewEvent::FullscreenChanged(fullscreen == 1));
        }

        fn on_tooltip(
            &self,
            _browser: Option<&mut Browser>,
            text: Option<&mut CefString>,
        ) -> ::std::os::raw::c_int {
            let Some(view) = self.view.upgrade() else { return 0 };
            let text = text.map(|t| t.to_string()).unwrap_or_default();
            view.push(ViewEvent::Tooltip(non_empty(text)));
            1
        }

        fn on_status_message(&self, _browser: Option<&mut Browser>, value: Option<&CefString>) {
            let Some(view) = self.view.upgrade() else { return };
            view.push(ViewEvent::StatusMessage(non_empty(cef_str(value))));
        }

        fn on_console_message(
            &self,
            _browser: Option<&mut Browser>,
            level: LogSeverity,
            message: Option<&CefString>,
            source: Option<&CefString>,
            line: ::std::os::raw::c_int,
        ) -> ::std::os::raw::c_int {
            let Some(view) = self.view.upgrade() else { return 0 };
            let level: cef::sys::cef_log_severity_t = level.into();
            view.push(ViewEvent::ConsoleMessage {
                level: level as i32,
                message: cef_str(message),
                source: cef_str(source),
                line,
            });
            0
        }

        fn on_loading_progress_change(&self, _browser: Option<&mut Browser>, progress: f64) {
            let Some(view) = self.view.upgrade() else { return };
            view.push(ViewEvent::LoadProgress(progress));
        }

        fn on_cursor_change(
            &self,
            _browser: Option<&mut Browser>,
            _cursor: ::std::os::raw::c_ulong,
            type_: CursorType,
            _custom_cursor_info: Option<&CursorInfo>,
        ) -> ::std::os::raw::c_int {
            let Some(view) = self.view.upgrade() else { return 0 };
            view.push(ViewEvent::CursorChanged(cursor_kind(type_)));
            1
        }

        fn on_media_access_change(
            &self,
            _browser: Option<&mut Browser>,
            has_video_access: ::std::os::raw::c_int,
            has_audio_access: ::std::os::raw::c_int,
        ) {
            let Some(view) = self.view.upgrade() else { return };
            view.push(ViewEvent::MediaAccessChanged {
                video: has_video_access == 1,
                audio: has_audio_access == 1,
            });
        }
    }
}

wrap_download_image_callback! {
    pub(crate) struct FaviconCallback {
        view: Weak<ViewInner>,
    }

    impl DownloadImageCallback {
        fn on_download_image_finished(
            &self,
            _image_url: Option<&CefString>,
            _http_status_code: ::std::os::raw::c_int,
            image: Option<&mut Image>,
        ) {
            log::debug!("favicon download finished: status {_http_status_code}, image {}", image.is_some());
            let (Some(view), Some(image)) = (self.view.upgrade(), image) else { return };
            if image.is_empty() == 1 {
                return;
            }
            let mut w = 0;
            let mut h = 0;
            let Some(bitmap) = image.as_bitmap(
                view.scale.get(),
                ColorType::BGRA_8888,
                AlphaType::PREMULTIPLIED,
                Some(&mut w),
                Some(&mut h),
            ) else {
                return;
            };
            let mut bytes = vec![0u8; bitmap.size()];
            bitmap.data(Some(&mut bytes), 0);
            if w > 0 && h > 0 && bytes.len() >= (w * h * 4) as usize {
                view.push(ViewEvent::Favicon(Frame { width: w as u32, height: h as u32, bgra: bytes }));
            }
        }
    }
}

// ---- Loading --------------------------------------------------------------

wrap_load_handler! {
    pub(crate) struct LoadHandlerImpl {
        view: Weak<ViewInner>,
    }

    impl LoadHandler {
        fn on_loading_state_change(
            &self,
            _browser: Option<&mut Browser>,
            is_loading: ::std::os::raw::c_int,
            can_go_back: ::std::os::raw::c_int,
            can_go_forward: ::std::os::raw::c_int,
        ) {
            let Some(view) = self.view.upgrade() else { return };
            view.push(ViewEvent::LoadingStateChanged {
                loading: is_loading == 1,
                can_go_back: can_go_back == 1,
                can_go_forward: can_go_forward == 1,
            });
        }

        fn on_load_start(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut cef::Frame>,
            _transition_type: TransitionType,
        ) {
            if let Some(frame) = frame {
                crate::appearance::on_load_start(frame);
            }
        }

        fn on_load_end(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut cef::Frame>,
            http_status_code: ::std::os::raw::c_int,
        ) {
            let Some(view) = self.view.upgrade() else { return };
            if frame.map(|f| f.is_main() == 1).unwrap_or(false) {
                view.push(ViewEvent::LoadEnd { http_status: http_status_code });
            }
        }

        fn on_load_error(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut cef::Frame>,
            error_code: Errorcode,
            error_text: Option<&CefString>,
            failed_url: Option<&CefString>,
        ) {
            let Some(view) = self.view.upgrade() else { return };
            if !frame.map(|f| f.is_main() == 1).unwrap_or(false) {
                return;
            }
            let code: cef::sys::cef_errorcode_t = error_code.into();
            let code = code as i32;
            // ERR_ABORTED: the user stopped or replaced the navigation.
            if code == -3 {
                return;
            }
            view.push(ViewEvent::LoadError {
                code,
                text: cef_str(error_text),
                url: cef_str(failed_url),
            });
        }
    }
}

// ---- Lifetime & popups ------------------------------------------------------

wrap_life_span_handler! {
    pub(crate) struct LifeSpanHandlerImpl {
        view: Weak<ViewInner>,
    }

    impl LifeSpanHandler {
        fn on_before_popup(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut cef::Frame>,
            _popup_id: ::std::os::raw::c_int,
            target_url: Option<&CefString>,
            _target_frame_name: Option<&CefString>,
            target_disposition: WindowOpenDisposition,
            user_gesture: ::std::os::raw::c_int,
            _popup_features: Option<&PopupFeatures>,
            window_info: Option<&mut WindowInfo>,
            client: Option<&mut Option<Client>>,
            settings: Option<&mut BrowserSettings>,
            _extra_info: Option<&mut Option<DictionaryValue>>,
            _no_javascript_access: Option<&mut ::std::os::raw::c_int>,
        ) -> ::std::os::raw::c_int {
            let Some(opener) = self.view.upgrade() else { return 1 };
            // Let Chromium create the popup (so window.opener and postMessage
            // keep working for sign-in flows), but render it offscreen through
            // a Mori client. The chrome decides whether it becomes a tab.
            let (_, new_client) = pending_view(&opener, Announce {
                opener: Rc::downgrade(&opener),
                url: cef_str(target_url),
                disposition: target_disposition.into(),
                user_gesture: user_gesture == 1,
            });
            if let Some(info) = window_info {
                info.windowless_rendering_enabled = true as _;
                info.parent_window = 0;
            }
            if let Some(settings) = settings {
                *settings = browser_settings();
            }
            if let Some(client) = client {
                *client = Some(new_client);
            }
            0
        }

        fn on_before_popup_aborted(&self, _browser: Option<&mut Browser>, _popup_id: ::std::os::raw::c_int) {
            // The most recent pending popup was abandoned by Chromium.
            if let Some(inner) = PENDING.with(|p| p.borrow_mut().pop()) {
                ANNOUNCE.with(|a| a.borrow_mut().retain(|(id, _)| *id != inner.id));
            }
        }

        fn on_after_created(&self, browser: Option<&mut Browser>) {
            let (Some(view), Some(browser)) = (self.view.upgrade(), browser) else { return };
            *view.browser.borrow_mut() = Some(browser.clone());
            crate::devtools::attach(&view);
            crate::appearance::attach(&view);
            if let Some(url) = view.pending_url.borrow_mut().take()
                && let Some(frame) = browser.main_frame()
            {
                frame.load_url(Some(&CefString::from(url.as_str())));
            }
            if view.hidden.get()
                && let Some(host) = browser.host()
            {
                host.was_hidden(1);
            }
            view.push(ViewEvent::Created);

            // Announce Chromium-created views to their opener and release the
            // pending strong reference into the event.
            let announce = ANNOUNCE.with(|a| {
                let mut a = a.borrow_mut();
                a.iter().position(|(id, _)| *id == view.id).map(|i| a.remove(i).1)
            });
            let strong = PENDING.with(|p| {
                let mut p = p.borrow_mut();
                p.iter().position(|v| v.id == view.id).map(|i| p.remove(i))
            });
            if let (Some(announce), Some(strong)) = (announce, strong) {
                match announce.opener.upgrade() {
                    Some(opener) => opener.push(ViewEvent::NewView {
                        view: WebView::from_inner(strong),
                        url: announce.url,
                        disposition: announce.disposition,
                        user_gesture: announce.user_gesture,
                    }),
                    None => WebView::from_inner(strong).close(),
                }
            }
        }

        fn do_close(&self, _browser: Option<&mut Browser>) -> ::std::os::raw::c_int {
            if let Some(view) = self.view.upgrade()
                && !view.closing.get()
            {
                // The page called window.close(); let the chrome decide.
                view.push(ViewEvent::CloseRequested);
            }
            0
        }

        fn on_before_close(&self, _browser: Option<&mut Browser>) {
            let Some(view) = self.view.upgrade() else { return };
            view.closed.set(true);
            *view.devtools_registration.borrow_mut() = None;
            *view.browser.borrow_mut() = None;
            view.push(ViewEvent::Closed);
        }
    }
}

// ---- Requests -------------------------------------------------------------

wrap_request_handler! {
    pub(crate) struct RequestHandlerImpl {
        view: Weak<ViewInner>,
    }

    impl RequestHandler {
        fn on_open_urlfrom_tab(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut cef::Frame>,
            target_url: Option<&CefString>,
            target_disposition: WindowOpenDisposition,
            user_gesture: ::std::os::raw::c_int,
        ) -> ::std::os::raw::c_int {
            let Some(view) = self.view.upgrade() else { return 0 };
            let disposition: Disposition = target_disposition.into();
            if disposition == Disposition::CurrentTab {
                return 0;
            }
            // Middle-click / ctrl-click: open in a fresh view.
            let url = cef_str(target_url);
            let (w, h) = view.size.get();
            let new_view = WebView::new(&url, (w, h), view.scale.get());
            if matches!(disposition, Disposition::BackgroundTab) {
                new_view.set_hidden(true);
            }
            view.push(ViewEvent::NewView {
                view: new_view,
                url,
                disposition,
                user_gesture: user_gesture == 1,
            });
            1
        }

        fn on_render_process_terminated(
            &self,
            _browser: Option<&mut Browser>,
            _status: TerminationStatus,
            _error_code: ::std::os::raw::c_int,
            _error_string: Option<&CefString>,
        ) {
            if let Some(view) = self.view.upgrade() {
                view.push(ViewEvent::RenderProcessGone);
            }
        }
    }
}

// ---- JavaScript dialogs ------------------------------------------------------

wrap_jsdialog_handler! {
    pub(crate) struct JsDialogHandlerImpl {
        view: Weak<ViewInner>,
    }

    impl JsdialogHandler {
        fn on_jsdialog(
            &self,
            _browser: Option<&mut Browser>,
            origin_url: Option<&CefString>,
            dialog_type: JsdialogType,
            message_text: Option<&CefString>,
            default_prompt_text: Option<&CefString>,
            callback: Option<&mut JsdialogCallback>,
            _suppress_message: Option<&mut ::std::os::raw::c_int>,
        ) -> ::std::os::raw::c_int {
            let (Some(view), Some(callback)) = (self.view.upgrade(), callback) else { return 0 };
            let kind = match dialog_type {
                JsdialogType::CONFIRM => DialogKind::Confirm,
                JsdialogType::PROMPT => DialogKind::Prompt,
                _ => DialogKind::Alert,
            };
            let id = DialogId(next_token());
            view.dialogs.borrow_mut().insert(id, callback.clone());
            view.push(ViewEvent::JsDialog(JsDialog {
                id,
                kind,
                origin: cef_str(origin_url),
                message: cef_str(message_text),
                default_prompt: cef_str(default_prompt_text),
            }));
            1
        }

        fn on_before_unload_dialog(
            &self,
            _browser: Option<&mut Browser>,
            message_text: Option<&CefString>,
            _is_reload: ::std::os::raw::c_int,
            callback: Option<&mut JsdialogCallback>,
        ) -> ::std::os::raw::c_int {
            let (Some(view), Some(callback)) = (self.view.upgrade(), callback) else { return 0 };
            let id = DialogId(next_token());
            view.dialogs.borrow_mut().insert(id, callback.clone());
            view.push(ViewEvent::JsDialog(JsDialog {
                id,
                kind: DialogKind::BeforeUnload,
                origin: String::new(),
                message: cef_str(message_text),
                default_prompt: String::new(),
            }));
            1
        }

        fn on_reset_dialog_state(&self, _browser: Option<&mut Browser>) {
            let Some(view) = self.view.upgrade() else { return };
            view.dialogs.borrow_mut().clear();
            view.push(ViewEvent::DialogsReset);
        }
    }
}

// ---- Permissions ------------------------------------------------------------

fn permission_kinds(mask: u32) -> Vec<PermissionKind> {
    use cef::sys::cef_permission_request_types_t as T;
    let table: &[(T, PermissionKind)] = &[
        (T::CEF_PERMISSION_TYPE_CAMERA_STREAM, PermissionKind::Camera),
        (T::CEF_PERMISSION_TYPE_MIC_STREAM, PermissionKind::Microphone),
        (T::CEF_PERMISSION_TYPE_GEOLOCATION, PermissionKind::Geolocation),
        (T::CEF_PERMISSION_TYPE_NOTIFICATIONS, PermissionKind::Notifications),
        (T::CEF_PERMISSION_TYPE_CLIPBOARD, PermissionKind::Clipboard),
        (T::CEF_PERMISSION_TYPE_MIDI_SYSEX, PermissionKind::Midi),
        (T::CEF_PERMISSION_TYPE_STORAGE_ACCESS, PermissionKind::Storage),
        (T::CEF_PERMISSION_TYPE_WINDOW_MANAGEMENT, PermissionKind::WindowManagement),
        (T::CEF_PERMISSION_TYPE_LOCAL_FONTS, PermissionKind::LocalFonts),
        (T::CEF_PERMISSION_TYPE_IDLE_DETECTION, PermissionKind::IdleDetection),
    ];
    let mut kinds: Vec<PermissionKind> = table
        .iter()
        .filter(|(bit, _)| mask & (*bit as u32) != 0)
        .map(|(_, k)| *k)
        .collect();
    if kinds.is_empty() && mask != 0 {
        kinds.push(PermissionKind::Other);
    }
    kinds
}

fn media_kinds(mask: u32) -> Vec<PermissionKind> {
    use cef::sys::cef_media_access_permission_types_t as M;
    let mut kinds = Vec::new();
    if mask & (M::CEF_MEDIA_PERMISSION_DEVICE_VIDEO_CAPTURE as u32) != 0 {
        kinds.push(PermissionKind::Camera);
    }
    if mask & (M::CEF_MEDIA_PERMISSION_DEVICE_AUDIO_CAPTURE as u32) != 0 {
        kinds.push(PermissionKind::Microphone);
    }
    if mask
        & (M::CEF_MEDIA_PERMISSION_DESKTOP_VIDEO_CAPTURE as u32
            | M::CEF_MEDIA_PERMISSION_DESKTOP_AUDIO_CAPTURE as u32)
        != 0
    {
        kinds.push(PermissionKind::ScreenCapture);
    }
    kinds
}

wrap_permission_handler! {
    pub(crate) struct PermissionHandlerImpl {
        view: Weak<ViewInner>,
    }

    impl PermissionHandler {
        fn on_request_media_access_permission(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut cef::Frame>,
            requesting_origin: Option<&CefString>,
            requested_permissions: u32,
            callback: Option<&mut MediaAccessCallback>,
        ) -> ::std::os::raw::c_int {
            let (Some(view), Some(callback)) = (self.view.upgrade(), callback) else { return 0 };
            let id = PermissionId(next_token());
            view.permissions
                .borrow_mut()
                .insert(id, PendingPermission::Media(callback.clone(), requested_permissions));
            view.push(ViewEvent::PermissionPrompt(PermissionRequest {
                id,
                origin: cef_str(requesting_origin),
                kinds: media_kinds(requested_permissions),
            }));
            1
        }

        fn on_show_permission_prompt(
            &self,
            _browser: Option<&mut Browser>,
            _prompt_id: u64,
            requesting_origin: Option<&CefString>,
            requested_permissions: u32,
            callback: Option<&mut PermissionPromptCallback>,
        ) -> ::std::os::raw::c_int {
            let (Some(view), Some(callback)) = (self.view.upgrade(), callback) else { return 0 };
            let id = PermissionId(next_token());
            view.permissions
                .borrow_mut()
                .insert(id, PendingPermission::Prompt(callback.clone()));
            view.push(ViewEvent::PermissionPrompt(PermissionRequest {
                id,
                origin: cef_str(requesting_origin),
                kinds: permission_kinds(requested_permissions),
            }));
            1
        }
    }
}

// ---- Context menus ------------------------------------------------------------

wrap_context_menu_handler! {
    pub(crate) struct ContextMenuHandlerImpl {
        view: Weak<ViewInner>,
    }

    impl ContextMenuHandler {
        fn on_before_context_menu(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut cef::Frame>,
            params: Option<&mut ContextMenuParams>,
            model: Option<&mut MenuModel>,
        ) {
            // Mori draws its own context menu; suppress Chromium's.
            if let Some(model) = model {
                model.clear();
            }
            let (Some(view), Some(params)) = (self.view.upgrade(), params) else { return };
            let flags: cef::sys::cef_context_menu_edit_state_flags_t = params.edit_state_flags().into();
            let flags = flags.0;
            use cef::sys::cef_context_menu_edit_state_flags_t as E;
            let has = |f: E| flags & f.0 != 0;
            let media_type: cef::sys::cef_context_menu_media_type_t = params.media_type().into();
            let source = non_empty(userfree(&params.source_url()));
            let is_image = media_type == cef::sys::cef_context_menu_media_type_t::CM_MEDIATYPE_IMAGE;
            let is_media = matches!(
                media_type,
                cef::sys::cef_context_menu_media_type_t::CM_MEDIATYPE_VIDEO
                    | cef::sys::cef_context_menu_media_type_t::CM_MEDIATYPE_AUDIO
            );
            let mut suggestions = CefStringList::new();
            params.dictionary_suggestions(Some(&mut suggestions));
            let info = ContextMenuInfo {
                x: params.xcoord(),
                y: params.ycoord(),
                page_url: userfree(&params.page_url()),
                link_url: non_empty(userfree(&params.link_url())),
                link_text: non_empty(userfree(&params.title_text())),
                image_url: if is_image { source.clone() } else { None },
                media_url: if is_media { source } else { None },
                selection: non_empty(userfree(&params.selection_text())),
                is_editable: params.is_editable() == 1,
                can_undo: has(E::CM_EDITFLAG_CAN_UNDO),
                can_redo: has(E::CM_EDITFLAG_CAN_REDO),
                can_cut: has(E::CM_EDITFLAG_CAN_CUT),
                can_copy: has(E::CM_EDITFLAG_CAN_COPY),
                can_paste: has(E::CM_EDITFLAG_CAN_PASTE),
                can_select_all: has(E::CM_EDITFLAG_CAN_SELECT_ALL),
                misspelled_word: non_empty(userfree(&params.misspelled_word())),
                suggestions: suggestions.into_iter().map(|s| s.to_string()).collect(),
            };
            view.push(ViewEvent::ContextMenu(info));
        }
    }
}

// ---- Find -----------------------------------------------------------------

wrap_find_handler! {
    pub(crate) struct FindHandlerImpl {
        view: Weak<ViewInner>,
    }

    impl FindHandler {
        fn on_find_result(
            &self,
            _browser: Option<&mut Browser>,
            _identifier: ::std::os::raw::c_int,
            count: ::std::os::raw::c_int,
            _selection_rect: Option<&Rect>,
            active_match_ordinal: ::std::os::raw::c_int,
            final_update: ::std::os::raw::c_int,
        ) {
            let Some(view) = self.view.upgrade() else { return };
            view.push(ViewEvent::FindResult {
                count,
                active: active_match_ordinal,
                final_update: final_update == 1,
            });
        }
    }
}
