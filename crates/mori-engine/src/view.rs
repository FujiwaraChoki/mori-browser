//! Offscreen web views.
//!
//! A [`WebView`] owns one CEF browser rendered offscreen. Chromium callbacks
//! update the view's shared state and queue [`ViewEvent`]s; the UI drains
//! them with [`WebView::take_events`] after each [`crate::pump`].

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet, VecDeque},
    rc::{Rc, Weak},
};

use cef::*;

use crate::{DownloadId, client, devtools::EvalCallback, keys::KeyInput};

/// Stable identifier for a view, unique for the life of the process.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ViewId(pub u64);

/// A full-view paint from Chromium, BGRA, premultiplied, `width * height * 4`.
#[derive(Clone, Debug, Default)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

/// A paint of a `<select>` dropdown or similar popup widget, positioned in
/// view coordinates (logical pixels).
#[derive(Clone, Debug, Default)]
pub struct PopupFrame {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub frame: Frame,
}

/// Where Chromium wants a new page opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disposition {
    CurrentTab,
    ForegroundTab,
    BackgroundTab,
    Popup,
    Window,
    Other,
}

impl From<WindowOpenDisposition> for Disposition {
    fn from(d: WindowOpenDisposition) -> Self {
        match d {
            WindowOpenDisposition::CURRENT_TAB => Disposition::CurrentTab,
            WindowOpenDisposition::NEW_FOREGROUND_TAB
            | WindowOpenDisposition::SINGLETON_TAB
            | WindowOpenDisposition::SWITCH_TO_TAB => Disposition::ForegroundTab,
            WindowOpenDisposition::NEW_BACKGROUND_TAB => Disposition::BackgroundTab,
            WindowOpenDisposition::NEW_POPUP => Disposition::Popup,
            WindowOpenDisposition::NEW_WINDOW => Disposition::Window,
            _ => Disposition::Other,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CursorKind {
    #[default]
    Arrow,
    Pointer,
    Text,
    Crosshair,
    Wait,
    Help,
    Move,
    NotAllowed,
    Grab,
    Grabbing,
    ResizeEw,
    ResizeNs,
    ResizeNesw,
    ResizeNwse,
    ResizeCol,
    ResizeRow,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DialogId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogKind {
    Alert,
    Confirm,
    Prompt,
    BeforeUnload,
}

#[derive(Clone, Debug)]
pub struct JsDialog {
    pub id: DialogId,
    pub kind: DialogKind,
    pub origin: String,
    pub message: String,
    pub default_prompt: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PermissionId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FileDialogId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileDialogMode {
    Open,
    OpenMultiple,
    OpenFolder,
    Save,
}

/// A page asked the user to pick files (`<input type=file>`,
/// `showOpenFilePicker`, ...). Answer with [`WebView::resolve_file_dialog`].
#[derive(Clone, Debug)]
pub struct FileDialogRequest {
    pub id: FileDialogId,
    pub mode: FileDialogMode,
    pub title: String,
    pub default_path: String,
    /// MIME types (`image/*`) and extensions (`.png`) the page accepts.
    pub accept: Vec<String>,
}

/// Geometry a page asked for in `window.open(url, name, features)`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PopupFeatures {
    pub width: Option<i32>,
    pub height: Option<i32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PermissionKind {
    Camera,
    Microphone,
    Geolocation,
    Notifications,
    Clipboard,
    Midi,
    Sensors,
    ScreenCapture,
    Storage,
    WindowManagement,
    LocalFonts,
    IdleDetection,
    Other,
}

#[derive(Clone, Debug)]
pub struct PermissionRequest {
    pub id: PermissionId,
    pub origin: String,
    pub kinds: Vec<PermissionKind>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AuthId(pub u64);

/// A server or proxy asked for HTTP credentials (Basic/Digest/NTLM).
#[derive(Clone, Debug)]
pub struct AuthRequest {
    pub id: AuthId,
    /// Origin of the page that made the request.
    pub origin: String,
    pub host: String,
    pub port: i32,
    pub realm: String,
    pub scheme: String,
    pub is_proxy: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CertErrorId(pub u64);

/// A navigation hit an invalid TLS certificate and waits for a decision.
#[derive(Clone, Debug)]
pub struct CertificateError {
    pub id: CertErrorId,
    pub url: String,
    /// Chromium net error code, e.g. -202 (`ERR_CERT_AUTHORITY_INVALID`).
    pub code: i32,
}

/// A key press the page did not consume (no `preventDefault`), reported so
/// the chrome can run a browser shortcut bound to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnhandledKey {
    pub windows_key_code: i32,
    /// `keys::MOD_*` flags.
    pub modifiers: u32,
}

/// What the user right-clicked on.
#[derive(Clone, Debug, Default)]
pub struct ContextMenuInfo {
    /// Position in view coordinates (logical pixels).
    pub x: i32,
    pub y: i32,
    pub page_url: String,
    pub link_url: Option<String>,
    pub link_text: Option<String>,
    pub image_url: Option<String>,
    pub media_url: Option<String>,
    pub selection: Option<String>,
    pub is_editable: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    pub can_cut: bool,
    pub can_copy: bool,
    pub can_paste: bool,
    pub can_select_all: bool,
    pub misspelled_word: Option<String>,
    pub suggestions: Vec<String>,
}

/// Something the chrome should react to.
#[derive(Debug)]
pub enum ViewEvent {
    /// The browser was created; navigation methods now take effect.
    Created,
    AddressChanged(String),
    TitleChanged(String),
    FaviconUrlChanged(Option<String>),
    /// A decoded favicon, BGRA.
    Favicon(Frame),
    LoadingStateChanged {
        loading: bool,
        can_go_back: bool,
        can_go_forward: bool,
    },
    LoadProgress(f64),
    LoadEnd {
        http_status: i32,
    },
    LoadError {
        code: i32,
        text: String,
        url: String,
    },
    /// A new paint is available via [`WebView::frame`].
    Painted,
    PopupWidgetChanged,
    CursorChanged(CursorKind),
    StatusMessage(Option<String>),
    Tooltip(Option<String>),
    FullscreenChanged(bool),
    /// Chromium opened a new page (target=_blank, window.open, middle click).
    /// The view is already alive and loading; the chrome decides where it goes.
    NewView {
        view: WebView,
        url: String,
        disposition: Disposition,
        user_gesture: bool,
        /// Requested window geometry for `window.open` popups.
        features: PopupFeatures,
    },
    /// The page asked to be closed (`window.close()`), or the browser is gone.
    CloseRequested,
    Closed,
    FindResult {
        count: i32,
        active: i32,
        final_update: bool,
    },
    ContextMenu(ContextMenuInfo),
    JsDialog(JsDialog),
    /// A dialog that was being shown is no longer relevant (navigation, etc.).
    DialogsReset,
    PermissionPrompt(PermissionRequest),
    PermissionDismissed(PermissionId),
    FileDialog(FileDialogRequest),
    AuthRequired(AuthRequest),
    CertificateError(CertificateError),
    UnhandledKey(UnhandledKey),
    TextSelectionChanged(String),
    /// Composition character bounds in view coordinates, for IME placement.
    ImeCompositionBounds(Vec<(i32, i32, i32, i32)>),
    AudioStateChanged {
        audible: bool,
    },
    MediaAccessChanged {
        video: bool,
        audio: bool,
    },
    RenderProcessGone,
    /// The page called `window.print()`.
    PrintRequested,
    ConsoleMessage {
        level: i32,
        message: String,
        source: String,
        line: i32,
    },
}

pub(crate) struct ViewInner {
    pub id: ViewId,
    pub browser: RefCell<Option<Browser>>,
    pub client: RefCell<Option<Client>>,
    pub events: RefCell<VecDeque<ViewEvent>>,
    /// View size in logical pixels.
    pub size: Cell<(i32, i32)>,
    pub scale: Cell<f32>,
    pub hidden: Cell<bool>,
    pub frame: RefCell<Option<Frame>>,
    pub frame_generation: Cell<u64>,
    pub popup: RefCell<Option<PopupFrame>>,
    pub popup_visible: Cell<bool>,
    pub pending_url: RefCell<Option<String>>,
    pub dialogs: RefCell<HashMap<DialogId, JsdialogCallback>>,
    pub permissions: RefCell<HashMap<PermissionId, PendingPermission>>,
    pub auths: RefCell<HashMap<AuthId, AuthCallback>>,
    pub cert_errors: RefCell<HashMap<CertErrorId, Callback>>,
    pub evals: RefCell<HashMap<i32, EvalCallback>>,
    pub file_dialogs: RefCell<HashMap<FileDialogId, FileDialogCallback>>,
    pub devtools_registration: RefCell<Option<Registration>>,
    /// Where protocol messages for an attached DevTools frontend go.
    pub devtools_sink: RefCell<Option<crate::devtools::Sink>>,
    /// An in-page drag Chromium started (see `drag.rs`).
    pub drag: RefCell<Option<crate::drag::DragSession>>,
    /// Downloads this view started that are still running. Chromium stops
    /// reporting downloads whose browser is gone, so a view closed while it
    /// has some is parked (hidden, blank) until they finish.
    pub active_downloads: RefCell<HashSet<DownloadId>>,
    pub parked: Cell<bool>,
    /// A `print_to_pdf` is running (its print start is not the page's).
    pub printing_pdf: Cell<bool>,
    /// Between `browser_host_create_browser` and `on_after_created`.
    pub creating: Cell<bool>,
    pub closing: Cell<bool>,
    pub closed: Cell<bool>,
    /// Screen origin of the view (logical px), reported to Chromium so popups
    /// and `screenX` land in the right place.
    pub screen_origin: Cell<(i32, i32)>,
}

pub(crate) enum PendingPermission {
    /// Chromium's prompt id, so a withdrawn prompt can be matched.
    Prompt(PermissionPromptCallback, u64),
    Media(MediaAccessCallback, u32),
}

thread_local! {
    static VIEWS: RefCell<HashMap<ViewId, Weak<ViewInner>>> = RefCell::new(HashMap::new());
    static DIRTY: RefCell<HashSet<ViewId>> = RefCell::new(HashSet::new());
    /// Views asked to close (or parked for their downloads). They must
    /// outlive their last `WebView` handle until Chromium has destroyed the
    /// browser, or the browser would run on with nothing to close it.
    static CLOSING: RefCell<Vec<Rc<ViewInner>>> = const { RefCell::new(Vec::new()) };
    static NEXT_ID: Cell<u64> = const { Cell::new(1) };
    static NEXT_TOKEN: Cell<u64> = const { Cell::new(1) };
}

pub(crate) fn next_token() -> u64 {
    NEXT_TOKEN.with(|n| {
        let v = n.get();
        n.set(v + 1);
        v
    })
}

impl ViewInner {
    pub(crate) fn new(size: (i32, i32), scale: f32) -> Rc<Self> {
        let id = NEXT_ID.with(|n| {
            let v = n.get();
            n.set(v + 1);
            ViewId(v)
        });
        let inner = Rc::new(ViewInner {
            id,
            browser: RefCell::new(None),
            client: RefCell::new(None),
            events: RefCell::new(VecDeque::new()),
            size: Cell::new((size.0.max(1), size.1.max(1))),
            scale: Cell::new(scale.max(0.5)),
            hidden: Cell::new(false),
            frame: RefCell::new(None),
            frame_generation: Cell::new(0),
            popup: RefCell::new(None),
            popup_visible: Cell::new(false),
            pending_url: RefCell::new(None),
            dialogs: RefCell::new(HashMap::new()),
            permissions: RefCell::new(HashMap::new()),
            auths: RefCell::new(HashMap::new()),
            cert_errors: RefCell::new(HashMap::new()),
            evals: RefCell::new(HashMap::new()),
            file_dialogs: RefCell::new(HashMap::new()),
            devtools_registration: RefCell::new(None),
            devtools_sink: RefCell::new(None),
            drag: RefCell::new(None),
            active_downloads: RefCell::new(HashSet::new()),
            parked: Cell::new(false),
            printing_pdf: Cell::new(false),
            creating: Cell::new(false),
            closing: Cell::new(false),
            closed: Cell::new(false),
            screen_origin: Cell::new((0, 0)),
        });
        VIEWS.with(|v| v.borrow_mut().insert(id, Rc::downgrade(&inner)));
        inner
    }

    pub(crate) fn push(&self, event: ViewEvent) {
        self.events.borrow_mut().push_back(event);
        self.mark_dirty();
    }

    pub(crate) fn mark_dirty(&self) {
        DIRTY.with(|d| d.borrow_mut().insert(self.id));
    }

    pub(crate) fn host(&self) -> Option<BrowserHost> {
        self.browser.borrow().as_ref().and_then(|b| b.host())
    }

    pub(crate) fn main_frame(&self) -> Option<cef::Frame> {
        self.browser.borrow().as_ref().and_then(|b| b.main_frame())
    }

    pub(crate) fn focused_frame(&self) -> Option<cef::Frame> {
        self.browser
            .borrow()
            .as_ref()
            .and_then(|b| b.focused_frame().or_else(|| b.main_frame()))
    }

    /// Fail callbacks that can no longer be answered once the browser is
    /// going away. They run outside the borrows: they may call back in.
    pub(crate) fn fail_pending(&self, reason: &str) {
        let evals: Vec<_> = self.evals.borrow_mut().drain().map(|(_, cb)| cb).collect();
        for cb in evals {
            cb(Err(reason.to_string()));
        }
        let dialogs: Vec<_> = self.file_dialogs.borrow_mut().drain().map(|(_, cb)| cb).collect();
        for cb in dialogs {
            cb.cancel();
        }
    }

    /// Ask Chromium to destroy the browser. The view stays alive in
    /// `CLOSING` until `on_before_close` (or `on_after_created`, for a
    /// browser that did not exist yet) finishes the job.
    pub(crate) fn close_browser_now(self: &Rc<Self>) {
        self.closing.set(true);
        self.keep_alive();
        match self.host() {
            Some(host) => host.close_browser(true as _),
            None if self.creating.get() => {}
            None => {
                self.closed.set(true);
                release_closed(self.id);
            }
        }
    }

    /// Hold a strong reference until [`release_closed`].
    pub(crate) fn keep_alive(self: &Rc<Self>) {
        CLOSING.with(|c| {
            let mut c = c.borrow_mut();
            if !c.iter().any(|v| Rc::ptr_eq(v, self)) {
                c.push(self.clone());
            }
        });
    }
}

/// Drop the keep-alive reference of a view whose browser is gone.
pub(crate) fn release_closed(id: ViewId) {
    let released: Vec<Rc<ViewInner>> = CLOSING.with(|c| {
        let mut c = c.borrow_mut();
        let (gone, keep) = std::mem::take(&mut *c).into_iter().partition(|v| v.id == id);
        *c = keep;
        gone
    });
    // Dropped outside the `CLOSING` borrow: dropping may re-enter.
    drop(released);
}

/// Track a download a view started; once a parked view's last download
/// settles, close it for real.
pub(crate) fn note_download(view: &Rc<ViewInner>, id: DownloadId, running: bool) {
    let idle = {
        let mut active = view.active_downloads.borrow_mut();
        if running {
            active.insert(id);
        } else {
            active.remove(&id);
        }
        active.is_empty()
    };
    if idle && view.parked.replace(false) {
        log::debug!("downloads of parked view {:?} settled; closing it", view.id);
        view.close_browser_now();
    }
}

impl Drop for ViewInner {
    fn drop(&mut self) {
        VIEWS.with(|v| v.borrow_mut().remove(&self.id));
        // Every handle is gone (e.g. a `NewView` nobody adopted): nothing
        // could ever close this browser later, so close it now.
        if !self.closed.get()
            && let Some(host) = self.host()
        {
            host.close_browser(true as _);
        }
    }
}

/// Views that received events or paints since the last call.
pub fn take_dirty_views() -> Vec<ViewId> {
    DIRTY.with(|d| d.borrow_mut().drain().collect())
}

/// The live view showing Chromium browser `browser_id`.
pub(crate) fn view_for_browser(browser_id: i32) -> Option<Rc<ViewInner>> {
    VIEWS.with(|v| {
        v.borrow().values().filter_map(Weak::upgrade).find(|view| {
            view.browser
                .borrow()
                .as_ref()
                .is_some_and(|b| b.identifier() == browser_id)
        })
    })
}

/// Every view that is still alive.
pub(crate) fn all_views() -> Vec<Rc<ViewInner>> {
    VIEWS.with(|v| v.borrow().values().filter_map(Weak::upgrade).collect())
}

pub(crate) fn close_all_views() {
    let views: Vec<Rc<ViewInner>> =
        VIEWS.with(|v| v.borrow().values().filter_map(Weak::upgrade).collect());
    for view in views {
        // Quitting cancels running downloads, as in Chrome.
        view.active_downloads.borrow_mut().clear();
        if view.parked.replace(false) {
            view.close_browser_now();
        } else {
            WebView(view).close();
        }
    }
}

pub(crate) fn live_view_count() -> usize {
    VIEWS.with(|v| {
        v.borrow()
            .values()
            .filter_map(Weak::upgrade)
            .filter(|v| !v.closed.get() && (v.browser.borrow().is_some() || v.creating.get()))
            .count()
    })
}

/// A handle to an offscreen Chromium browser. Cheap to clone.
#[derive(Clone)]
pub struct WebView(pub(crate) Rc<ViewInner>);

impl std::fmt::Debug for WebView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "WebView({})", self.0.id.0)
    }
}

impl PartialEq for WebView {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl WebView {
    /// Create a new browser loading `url`. `size` is in logical pixels.
    pub fn new(url: &str, size: (i32, i32), scale: f32) -> WebView {
        let inner = ViewInner::new(size, scale);
        let mut client = client::build_client(&inner);
        *inner.client.borrow_mut() = Some(client.clone());

        let window_info = WindowInfo {
            windowless_rendering_enabled: true as _,
            ..Default::default()
        };
        let settings = client::browser_settings();
        inner.creating.set(true);
        let created = browser_host_create_browser(
            Some(&window_info),
            Some(&mut client),
            Some(&CefString::from(url)),
            Some(&settings),
            None,
            None,
        );
        if created != 1 {
            log::error!("failed to create browser for {url}");
            inner.creating.set(false);
        }
        WebView(inner)
    }

    /// Wrap a browser that Chromium created itself (popups); see
    /// [`ViewEvent::NewView`].
    pub(crate) fn from_inner(inner: Rc<ViewInner>) -> WebView {
        WebView(inner)
    }

    pub fn id(&self) -> ViewId {
        self.0.id
    }

    /// Look up a live view by id.
    pub fn by_id(id: ViewId) -> Option<WebView> {
        VIEWS.with(|v| v.borrow().get(&id).and_then(Weak::upgrade).map(WebView))
    }

    pub fn is_created(&self) -> bool {
        self.0.browser.borrow().is_some()
    }

    pub fn is_closed(&self) -> bool {
        self.0.closed.get()
    }

    /// The view was asked to close; its remaining events are of no interest.
    pub fn is_closing(&self) -> bool {
        self.0.closing.get() || self.0.parked.get()
    }

    pub fn take_events(&self) -> Vec<ViewEvent> {
        self.0.events.borrow_mut().drain(..).collect()
    }

    pub fn has_events(&self) -> bool {
        !self.0.events.borrow().is_empty()
    }

    /// Report this view from the next [`take_dirty_views`], e.g. once the
    /// chrome starts listening to it.
    pub fn mark_dirty(&self) {
        self.0.mark_dirty();
    }

    /// Monotonic counter bumped on each full-view paint.
    pub fn frame_generation(&self) -> u64 {
        self.0.frame_generation.get()
    }

    /// Take the most recent paint, leaving `None` until the next one.
    pub fn take_frame(&self) -> Option<Frame> {
        self.0.frame.borrow_mut().take()
    }

    pub fn popup(&self) -> Option<PopupFrame> {
        if self.0.popup_visible.get() {
            self.0.popup.borrow().clone()
        } else {
            None
        }
    }

    // ---- Navigation -------------------------------------------------------

    pub fn load_url(&self, url: &str) {
        match self.0.main_frame() {
            Some(frame) => frame.load_url(Some(&CefString::from(url))),
            None => *self.0.pending_url.borrow_mut() = Some(url.to_string()),
        }
    }

    pub fn url(&self) -> String {
        self.0
            .main_frame()
            .map(|f| CefString::from(&f.url()).to_string())
            .unwrap_or_default()
    }

    pub fn go_back(&self) {
        if let Some(b) = self.0.browser.borrow().as_ref() {
            b.go_back();
        }
    }

    pub fn go_forward(&self) {
        if let Some(b) = self.0.browser.borrow().as_ref() {
            b.go_forward();
        }
    }

    pub fn reload(&self, ignore_cache: bool) {
        if let Some(b) = self.0.browser.borrow().as_ref() {
            if ignore_cache {
                b.reload_ignore_cache();
            } else {
                b.reload();
            }
        }
    }

    pub fn stop(&self) {
        if let Some(b) = self.0.browser.borrow().as_ref() {
            b.stop_load();
        }
    }

    /// Whether the page is loading, or `None` before its browser exists.
    pub fn is_loading(&self) -> Option<bool> {
        self.0.browser.borrow().as_ref().map(|b| b.is_loading() == 1)
    }

    pub fn can_go_back(&self) -> bool {
        self.0
            .browser
            .borrow()
            .as_ref()
            .map(|b| b.can_go_back() == 1)
            .unwrap_or(false)
    }

    pub fn can_go_forward(&self) -> bool {
        self.0
            .browser
            .borrow()
            .as_ref()
            .map(|b| b.can_go_forward() == 1)
            .unwrap_or(false)
    }

    // ---- Geometry & visibility -------------------------------------------

    /// Resize the view (logical pixels) and/or change its device scale.
    pub fn resize(&self, width: i32, height: i32, scale: f32) {
        let size = (width.max(1), height.max(1));
        let scale_changed = (self.0.scale.get() - scale).abs() > f32::EPSILON;
        if size == self.0.size.get() && !scale_changed {
            return;
        }
        self.0.size.set(size);
        self.0.scale.set(scale.max(0.5));
        if let Some(host) = self.0.host() {
            if scale_changed {
                host.notify_screen_info_changed();
            }
            host.was_resized();
        }
    }

    pub fn size(&self) -> (i32, i32) {
        self.0.size.get()
    }

    pub fn set_screen_origin(&self, x: i32, y: i32) {
        self.0.screen_origin.set((x, y));
    }

    /// Hidden views stop painting and get throttled timers.
    pub fn set_hidden(&self, hidden: bool) {
        if self.0.hidden.replace(hidden) == hidden {
            return;
        }
        if let Some(host) = self.0.host() {
            host.was_hidden(hidden as _);
            if !hidden {
                host.invalidate(PaintElementType::VIEW);
            }
        }
    }

    pub fn invalidate(&self) {
        if let Some(host) = self.0.host() {
            host.invalidate(PaintElementType::VIEW);
        }
    }

    pub fn set_focus(&self, focused: bool) {
        if let Some(host) = self.0.host() {
            host.set_focus(focused as _);
        }
    }

    // ---- Input -----------------------------------------------------------

    pub fn send_mouse_move(&self, x: i32, y: i32, modifiers: u32, leave: bool) {
        // While the page drags something, the pointer drives the drag.
        if crate::drag::drag_move(&self.0, x, y, modifiers) {
            return;
        }
        if let Some(host) = self.0.host() {
            let ev = cef::MouseEvent { x, y, modifiers };
            host.send_mouse_move_event(Some(&ev), leave as _);
        }
    }

    pub fn send_mouse_click(
        &self,
        x: i32,
        y: i32,
        button: MouseButton,
        up: bool,
        click_count: i32,
        modifiers: u32,
    ) {
        if up && button == MouseButton::Left && crate::drag::drag_drop(&self.0, x, y, modifiers) {
            return;
        }
        if let Some(host) = self.0.host() {
            let ev = cef::MouseEvent { x, y, modifiers };
            let ty = match button {
                MouseButton::Left => MouseButtonType::LEFT,
                MouseButton::Middle => MouseButtonType::MIDDLE,
                MouseButton::Right => MouseButtonType::RIGHT,
            };
            host.send_mouse_click_event(Some(&ev), ty, up as _, click_count.max(1));
        }
    }

    /// Scroll by `(dx, dy)` pixels at `(x, y)`. Positive `dy` scrolls up.
    pub fn send_mouse_wheel(&self, x: i32, y: i32, dx: i32, dy: i32, modifiers: u32) {
        if let Some(host) = self.0.host() {
            let ev = cef::MouseEvent { x, y, modifiers };
            host.send_mouse_wheel_event(Some(&ev), dx, dy);
        }
    }

    pub fn send_key(&self, input: &KeyInput) {
        if let Some(host) = self.0.host() {
            for ev in input.to_cef_events() {
                host.send_key_event(Some(&ev));
            }
        }
    }

    /// Insert committed text (typed characters or an IME commit).
    pub fn insert_text(&self, text: &str) {
        let Some(host) = self.0.host() else { return };
        for unit in text.encode_utf16() {
            let ev = KeyEvent {
                size: std::mem::size_of::<KeyEvent>(),
                type_: KeyEventType::CHAR,
                windows_key_code: unit as _,
                character: unit,
                unmodified_character: unit,
                ..Default::default()
            };
            host.send_key_event(Some(&ev));
        }
    }

    pub fn ime_set_composition(&self, text: &str, cursor_utf16: Option<u32>) {
        let Some(host) = self.0.host() else { return };
        let len = text.encode_utf16().count() as u32;
        let underline = CompositionUnderline {
            size: std::mem::size_of::<CompositionUnderline>(),
            range: cef::Range { from: 0, to: len },
            color: 0,
            background_color: 0,
            thick: 0,
            style: Default::default(),
        };
        let cursor = cursor_utf16.unwrap_or(len);
        let selection = cef::Range {
            from: cursor,
            to: cursor,
        };
        let replacement = cef::Range {
            from: u32::MAX,
            to: u32::MAX,
        };
        host.ime_set_composition(
            Some(&CefString::from(text)),
            Some(&[underline]),
            Some(&replacement),
            Some(&selection),
        );
    }

    pub fn ime_commit(&self, text: &str) {
        let Some(host) = self.0.host() else { return };
        let replacement = cef::Range {
            from: u32::MAX,
            to: u32::MAX,
        };
        host.ime_commit_text(Some(&CefString::from(text)), Some(&replacement), 0);
    }

    pub fn ime_cancel(&self) {
        if let Some(host) = self.0.host() {
            host.ime_cancel_composition();
        }
    }

    pub fn ime_finish(&self) {
        if let Some(host) = self.0.host() {
            host.ime_finish_composing_text(0);
        }
    }

    pub fn send_capture_lost(&self) {
        if let Some(host) = self.0.host() {
            host.send_capture_lost_event();
        }
    }

    // ---- Editing ----------------------------------------------------------

    pub fn undo(&self) {
        if let Some(f) = self.0.focused_frame() {
            f.undo();
        }
    }
    pub fn redo(&self) {
        if let Some(f) = self.0.focused_frame() {
            f.redo();
        }
    }
    pub fn cut(&self) {
        if let Some(f) = self.0.focused_frame() {
            f.cut();
        }
    }
    pub fn copy(&self) {
        if let Some(f) = self.0.focused_frame() {
            f.copy();
        }
    }
    pub fn paste(&self) {
        if let Some(f) = self.0.focused_frame() {
            f.paste();
        }
    }
    pub fn paste_and_match_style(&self) {
        if let Some(f) = self.0.focused_frame() {
            f.paste_and_match_style();
        }
    }
    pub fn select_all(&self) {
        if let Some(f) = self.0.focused_frame() {
            f.select_all();
        }
    }
    pub fn replace_misspelling(&self, word: &str) {
        if let Some(host) = self.0.host() {
            host.replace_misspelling(Some(&CefString::from(word)));
        }
    }
    pub fn add_to_dictionary(&self, word: &str) {
        if let Some(host) = self.0.host() {
            host.add_word_to_dictionary(Some(&CefString::from(word)));
        }
    }

    // ---- Page tools -------------------------------------------------------

    /// Run `code` in the main frame without waiting for a result.
    pub fn execute_js(&self, code: &str) {
        if let Some(frame) = self.0.main_frame() {
            frame.execute_java_script(
                Some(&CefString::from(code)),
                Some(&CefString::from("mori://internal")),
                0,
            );
        }
    }

    /// Evaluate `expression` in the main frame and receive its JSON-encoded
    /// value (or an error string). Uses the DevTools protocol, so the result
    /// cannot be spoofed by page script. The page gets no user activation,
    /// so background work cannot unlock popups, autoplay or fullscreen.
    pub fn evaluate(
        &self,
        expression: &str,
        callback: impl FnOnce(Result<serde_json::Value, String>) + 'static,
    ) {
        crate::devtools::evaluate(&self.0, expression, false, Box::new(callback));
    }

    /// Like [`WebView::evaluate`], but runs as a user gesture. Only for
    /// scripts that carry out something the user just asked for (paste,
    /// a media button, an approved assistant action).
    pub fn evaluate_as_user(
        &self,
        expression: &str,
        callback: impl FnOnce(Result<serde_json::Value, String>) + 'static,
    ) {
        crate::devtools::evaluate(&self.0, expression, true, Box::new(callback));
    }

    /// Call a DevTools protocol method on the page; `callback` receives its
    /// `result` object or the error.
    pub fn devtools_call(
        &self,
        method: &str,
        params: serde_json::Value,
        callback: impl FnOnce(Result<serde_json::Value, String>) + 'static,
    ) {
        crate::devtools::call(&self.0, method, params, Box::new(callback));
    }

    pub fn find(&self, text: &str, forward: bool, match_case: bool, find_next: bool) {
        if let Some(host) = self.0.host() {
            host.find(
                Some(&CefString::from(text)),
                forward as _,
                match_case as _,
                find_next as _,
            );
        }
    }

    pub fn stop_finding(&self, clear_selection: bool) {
        if let Some(host) = self.0.host() {
            host.stop_finding(clear_selection as _);
        }
    }

    pub fn zoom_level(&self) -> f64 {
        self.0.host().map(|h| h.zoom_level()).unwrap_or(0.0)
    }

    /// Chromium zoom level: 0 = 100%, each step is 20% (log base 1.2).
    pub fn set_zoom_level(&self, level: f64) {
        if let Some(host) = self.0.host() {
            host.set_zoom_level(level);
        }
    }

    pub fn set_audio_muted(&self, muted: bool) {
        if let Some(host) = self.0.host() {
            host.set_audio_muted(muted as _);
        }
    }

    pub fn is_audio_muted(&self) -> bool {
        self.0
            .host()
            .map(|h| h.is_audio_muted() == 1)
            .unwrap_or(false)
    }

    pub fn exit_fullscreen(&self) {
        if let Some(host) = self.0.host() {
            host.exit_fullscreen(true as _);
        }
    }

    /// Render the page to a PDF at `path` through Chromium's print pipeline
    /// (the page's print CSS, with backgrounds). `done` gets whether it
    /// worked.
    pub fn print_to_pdf(&self, path: &std::path::Path, done: impl FnOnce(bool) + 'static) {
        crate::print::print_to_pdf(&self.0, path, Box::new(done));
    }

    /// Files dragged in from outside entered or moved over the page, at
    /// `(x, y)` in view coordinates.
    pub fn drag_files_over(&self, paths: &[std::path::PathBuf], x: i32, y: i32, modifiers: u32) {
        crate::drag::external_over(&self.0, paths, x, y, modifiers);
    }

    /// Dragged files left the page without being dropped.
    pub fn drag_files_leave(&self) {
        crate::drag::external_leave(&self.0);
    }

    /// Dragged files were dropped on the page at `(x, y)`.
    pub fn drop_files(&self, paths: &[std::path::PathBuf], x: i32, y: i32, modifiers: u32) {
        crate::drag::external_drop(&self.0, paths, x, y, modifiers);
    }

    pub fn start_download(&self, url: &str) {
        if let Some(host) = self.0.host() {
            host.start_download(Some(&CefString::from(url)));
        }
    }

    pub fn view_source(&self) {
        let url = self.url();
        if !url.is_empty() && !url.starts_with("view-source:") {
            self.load_url(&format!("view-source:{url}"));
        }
    }

    /// Connect a DevTools frontend: every protocol message from this page's
    /// session that Mori did not request itself (replies to the frontend,
    /// events) goes to `sink`. `None` disconnects.
    pub fn set_devtools_sink(&self, sink: Option<crate::DevToolsSink>) {
        crate::devtools::set_sink(&self.0, sink);
    }

    /// Send a raw DevTools protocol message (from the frontend) to the page.
    pub fn send_devtools_message(&self, message: &[u8]) -> bool {
        crate::devtools::send(&self.0, message)
    }

    // ---- Prompts ----------------------------------------------------------

    pub fn resolve_dialog(&self, id: DialogId, accept: bool, text: Option<&str>) {
        // Release the borrow first: Chromium re-enters on_reset_dialog_state
        // from inside cont().
        let cb = self.0.dialogs.borrow_mut().remove(&id);
        if let Some(cb) = cb {
            cb.cont(accept as _, text.map(CefString::from).as_ref());
        }
    }

    /// Answer an [`AuthRequest`]: `Some((user, password))` retries the
    /// request with credentials, `None` cancels it.
    pub fn resolve_auth(&self, id: AuthId, credentials: Option<(&str, &str)>) {
        let cb = self.0.auths.borrow_mut().remove(&id);
        if let Some(cb) = cb {
            match credentials {
                Some((user, password)) => {
                    cb.cont(Some(&CefString::from(user)), Some(&CefString::from(password)))
                }
                None => cb.cancel(),
            }
        }
    }

    /// Answer a [`CertificateError`]: `proceed` loads the page anyway (and
    /// Chromium remembers the exception for this session).
    pub fn resolve_certificate_error(&self, id: CertErrorId, proceed: bool) {
        let cb = self.0.cert_errors.borrow_mut().remove(&id);
        if let Some(cb) = cb {
            if proceed {
                cb.cont();
            } else {
                cb.cancel();
            }
        }
    }

    pub fn resolve_permission(&self, id: PermissionId, allow: bool) {
        let Some(pending) = self.0.permissions.borrow_mut().remove(&id) else {
            return;
        };
        match pending {
            PendingPermission::Prompt(cb, _) => cb.cont(if allow {
                PermissionRequestResult::ACCEPT
            } else {
                PermissionRequestResult::DENY
            }),
            PendingPermission::Media(cb, requested) => {
                cb.cont(if allow { requested } else { 0 })
            }
        }
    }

    /// Answer a [`ViewEvent::FileDialog`]; `None` (or no paths) cancels.
    pub fn resolve_file_dialog(&self, id: FileDialogId, paths: Option<Vec<std::path::PathBuf>>) {
        let Some(cb) = self.0.file_dialogs.borrow_mut().remove(&id) else { return };
        match paths.filter(|p| !p.is_empty()) {
            Some(paths) => {
                let mut list = CefStringList::new();
                for path in &paths {
                    list.append(&path.to_string_lossy());
                }
                cb.cont(Some(&mut list));
            }
            None => cb.cancel(),
        }
    }

    // ---- Lifetime ---------------------------------------------------------

    /// Ask the page to close, running its `beforeunload` handler first. If
    /// the page (or the user, through a [`DialogKind::BeforeUnload`] dialog)
    /// agrees, a [`ViewEvent::CloseRequested`] follows; otherwise nothing.
    pub fn request_close(&self) {
        if self.0.closing.get() {
            return;
        }
        match self.0.host() {
            Some(host) => host.close_browser(false as _),
            None => self.0.push(ViewEvent::CloseRequested),
        }
    }

    /// Close the browser. Emits [`ViewEvent::Closed`] once Chromium is done.
    ///
    /// A view with running downloads is parked instead (hidden, muted,
    /// navigated to a blank page) and closed once they finish, because
    /// Chromium cancels the downloads of a destroyed browser.
    pub fn close(&self) {
        if self.0.closing.get() || self.0.parked.get() {
            return;
        }
        // Deny anything still waiting on the user.
        let dialogs: Vec<_> = self.0.dialogs.borrow_mut().drain().collect();
        for (_, cb) in dialogs {
            cb.cont(0, None);
        }
        let ids: Vec<_> = self.0.permissions.borrow().keys().copied().collect();
        for id in ids {
            self.resolve_permission(id, false);
        }
        let auths: Vec<_> = self.0.auths.borrow_mut().drain().collect();
        for (_, cb) in auths {
            cb.cancel();
        }
        let certs: Vec<_> = self.0.cert_errors.borrow_mut().drain().collect();
        for (_, cb) in certs {
            cb.cancel();
        }
        self.0.fail_pending("view closed");
        crate::drag::cancel(&self.0);
        crate::devtools::set_sink(&self.0, None);
        if !self.0.active_downloads.borrow().is_empty() {
            log::debug!("parking view {:?} until its downloads finish", self.0.id);
            self.0.parked.set(true);
            self.0.keep_alive();
            self.set_hidden(true);
            self.set_audio_muted(true);
            self.load_url("about:blank");
            return;
        }
        self.0.close_browser_now();
    }
}

/// Copy the `dirty` rectangles (pixels) of a `width`x`height` BGRA frame
/// from `src` into `dst`, clipped to the frame. No rectangles, or one
/// covering the frame, copies everything.
pub(crate) fn copy_dirty_rects(dst: &mut [u8], src: &[u8], width: u32, height: u32, dirty: &[Rect]) {
    let (w, h) = (width as i32, height as i32);
    let stride = width as usize * 4;
    let covers = |r: &Rect| r.x <= 0 && r.y <= 0 && r.x + r.width >= w && r.y + r.height >= h;
    if dirty.is_empty() || dirty.iter().any(covers) {
        dst.copy_from_slice(src);
        return;
    }
    for r in dirty {
        let x0 = r.x.clamp(0, w) as usize;
        let x1 = r.x.saturating_add(r.width).clamp(0, w) as usize;
        let y0 = r.y.clamp(0, h) as usize;
        let y1 = r.y.saturating_add(r.height).clamp(0, h) as usize;
        if x0 >= x1 {
            continue;
        }
        for row in y0..y1 {
            let span = row * stride + x0 * 4..row * stride + x1 * 4;
            dst[span.clone()].copy_from_slice(&src[span]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, width: i32, height: i32) -> Rect {
        Rect { x, y, width, height }
    }

    #[test]
    fn copies_only_dirty_pixels() {
        let (w, h) = (4u32, 3u32);
        let src: Vec<u8> = (0..w * h * 4).map(|i| i as u8).collect();
        let mut dst = vec![0u8; src.len()];
        copy_dirty_rects(&mut dst, &src, w, h, &[rect(1, 1, 2, 1)]);
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let dirty = y == 1 && (1..3).contains(&x);
                let expected = if dirty { &src[i..i + 4] } else { &[0u8; 4][..] };
                assert_eq!(&dst[i..i + 4], expected, "pixel {x},{y}");
            }
        }
    }

    #[test]
    fn clips_and_handles_full_frames() {
        let (w, h) = (2u32, 2u32);
        let src = vec![7u8; 16];
        let mut dst = vec![0u8; 16];
        // Off-frame and empty rects are ignored.
        copy_dirty_rects(&mut dst, &src, w, h, &[rect(5, 5, 3, 3), rect(0, 0, 0, 2)]);
        assert_eq!(dst, vec![0u8; 16]);
        // Overhanging rect is clipped.
        copy_dirty_rects(&mut dst, &src, w, h, &[rect(1, -3, 10, 4)]);
        assert_eq!(&dst[4..8], &[7; 4]);
        assert_eq!(&dst[0..4], &[0; 4]);
        // A covering rect copies everything.
        copy_dirty_rects(&mut dst, &src, w, h, &[rect(-1, -1, 9, 9)]);
        assert_eq!(dst, src);
    }
}
