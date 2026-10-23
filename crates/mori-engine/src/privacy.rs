//! Privacy plumbing over CEF: the network hook Mori's content blocker
//! decides through, Do Not Track headers, permission content settings,
//! third-party cookie blocking, and access to cookies and the HTTP cache.
//!
//! Request callbacks run on Chromium's IO thread, so everything they touch
//! here is behind locks or atomics, never a view.

use std::{
    collections::HashMap,
    sync::{
        Arc, LazyLock, Mutex, RwLock,
        atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering},
    },
};

use cef::*;

use crate::{
    context::BrowsingContext,
    view::{PermissionKind, ViewEvent},
};

/// A network request as the content blocker sees it.
#[derive(Clone, Copy, Debug)]
pub struct RequestInfo<'a> {
    pub url: &'a str,
    /// The top-level page the request is made for (its first party).
    pub page_url: &'a str,
    /// The fetch destination in filter-list terms: `document`,
    /// `subdocument`, `script`, `image`, `stylesheet`, `font`, `media`,
    /// `xmlhttprequest`, `ping`, `object` or `other`.
    pub request_type: &'static str,
    /// Made by a page in a private (in-memory) context.
    pub private: bool,
}

/// Decides, on the network thread, whether a request is cancelled.
pub type RequestFilter = Arc<dyn Fn(&RequestInfo<'_>) -> bool + Send + Sync>;

static FILTER: RwLock<Option<RequestFilter>> = RwLock::new(None);
static DO_NOT_TRACK: AtomicBool = AtomicBool::new(false);

/// Install (or remove) the request filter consulted for every request.
pub fn set_request_filter(filter: Option<RequestFilter>) {
    if let Ok(mut slot) = FILTER.write() {
        *slot = filter;
    }
}

/// Send `DNT: 1` and `Sec-GPC: 1` with every request.
pub fn set_do_not_track(on: bool) {
    DO_NOT_TRACK.store(on, Ordering::Relaxed);
}

fn hooks_active() -> bool {
    DO_NOT_TRACK.load(Ordering::Relaxed) || FILTER.read().is_ok_and(|f| f.is_some())
}

fn request_type(t: ResourceType) -> &'static str {
    use cef::sys::cef_resource_type_t as R;
    let t: R = t.into();
    match t {
        R::RT_MAIN_FRAME | R::RT_NAVIGATION_PRELOAD_MAIN_FRAME => "document",
        R::RT_SUB_FRAME | R::RT_NAVIGATION_PRELOAD_SUB_FRAME => "subdocument",
        R::RT_STYLESHEET => "stylesheet",
        R::RT_SCRIPT | R::RT_WORKER | R::RT_SHARED_WORKER | R::RT_SERVICE_WORKER => "script",
        R::RT_IMAGE | R::RT_FAVICON => "image",
        R::RT_FONT_RESOURCE => "font",
        R::RT_OBJECT | R::RT_PLUGIN_RESOURCE => "object",
        R::RT_MEDIA => "media",
        R::RT_XHR => "xmlhttprequest",
        R::RT_PING | R::RT_CSP_REPORT => "ping",
        _ => "other",
    }
}

// ---- Blocked-request counts ---------------------------------------------------------

#[derive(Default)]
struct Counter {
    count: u32,
    /// A task telling the view is already on its way to the UI thread.
    notify_pending: bool,
}

static BLOCKED: LazyLock<Mutex<HashMap<i32, Counter>>> = LazyLock::new(Default::default);

/// Requests blocked on browser `browser_id`'s current page.
pub(crate) fn blocked_count(browser_id: i32) -> u32 {
    BLOCKED.lock().ok().and_then(|b| b.get(&browser_id).map(|c| c.count)).unwrap_or(0)
}

pub(crate) fn forget_browser(browser_id: i32) {
    if let Ok(mut b) = BLOCKED.lock() {
        b.remove(&browser_id);
    }
}

/// Update a browser's count (from the IO thread) and tell its view, at most
/// one task in flight per browser so bursts of blocked requests coalesce.
fn update_count(browser_id: i32, f: impl FnOnce(&mut u32)) {
    let post = {
        let Ok(mut blocked) = BLOCKED.lock() else { return };
        let counter = blocked.entry(browser_id).or_default();
        let before = counter.count;
        f(&mut counter.count);
        counter.count != before && !std::mem::replace(&mut counter.notify_pending, true)
    };
    if post {
        let mut task = BlockedTask::new(browser_id);
        post_task(ThreadId::UI, Some(&mut task));
    }
}

wrap_task! {
    pub(crate) struct BlockedTask {
        browser_id: ::std::os::raw::c_int,
    }

    impl Task {
        fn execute(&self) {
            let count = {
                let Ok(mut blocked) = BLOCKED.lock() else { return };
                let Some(counter) = blocked.get_mut(&self.browser_id) else { return };
                counter.notify_pending = false;
                counter.count
            };
            if let Some(view) = crate::view::view_for_browser(self.browser_id) {
                view.push(ViewEvent::ContentBlocked { count });
            }
        }
    }
}

// ---- The per-request handler ------------------------------------------------------

/// The resource handler for one request, or `None` when nothing needs to
/// look at requests. Called on the IO thread.
pub(crate) fn resource_request_handler(browser: Option<&mut Browser>, initiator: String) -> Option<ResourceRequestHandler> {
    if !hooks_active() {
        return None;
    }
    let private = browser.as_deref().is_some_and(is_private_browser);
    let browser_id = browser.map(|b| b.identifier()).unwrap_or(-1);
    Some(RequestHooks::new(browser_id, initiator, private))
}

/// Whether the browser's pages live in a private context: one that isn't
/// CEF's global context and keeps nothing on disk (see
/// [`BrowsingContext::private`]).
///
/// [`BrowsingContext::private`]: crate::context::BrowsingContext::private
fn is_private_browser(browser: &Browser) -> bool {
    let Some(ctx) = browser.host().and_then(|h| h.request_context()) else { return false };
    ctx.is_global() == 0 && userfree(&ctx.cache_path()).is_empty()
}

fn userfree(s: &CefStringUserfree) -> String {
    CefString::from(s).to_string()
}

wrap_resource_request_handler! {
    pub(crate) struct RequestHooks {
        browser_id: ::std::os::raw::c_int,
        // The origin that started the request, for requests without a
        // first party (service workers).
        initiator: String,
        private: bool,
    }

    impl ResourceRequestHandler {
        fn on_before_resource_load(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut cef::Frame>,
            request: Option<&mut Request>,
            _callback: Option<&mut Callback>,
        ) -> ReturnValue {
            let Some(request) = request else { return ReturnValue::CONTINUE };
            let kind = request_type(request.resource_type());
            if kind == "document" && self.browser_id >= 0 {
                update_count(self.browser_id, |c| *c = 0);
            }
            if DO_NOT_TRACK.load(Ordering::Relaxed) {
                for name in ["DNT", "Sec-GPC"] {
                    request.set_header_by_name(Some(&CefString::from(name)), Some(&CefString::from("1")), 1);
                }
            }
            let filter = FILTER.read().ok().and_then(|f| f.clone());
            if let Some(filter) = filter {
                let url = userfree(&request.url());
                let first_party = userfree(&request.first_party_for_cookies());
                let page_url = if first_party.is_empty() { self.initiator.as_str() } else { first_party.as_str() };
                if filter(&RequestInfo { url: &url, page_url, request_type: kind, private: self.private }) {
                    if self.browser_id >= 0 {
                        update_count(self.browser_id, |c| *c = c.saturating_add(1));
                    }
                    return ReturnValue::CANCEL;
                }
            }
            ReturnValue::CONTINUE
        }
    }
}

// ---- Permissions as content settings ----------------------------------------------

fn content_type(kind: PermissionKind) -> Option<ContentSettingTypes> {
    Some(match kind {
        PermissionKind::Geolocation => ContentSettingTypes::GEOLOCATION,
        PermissionKind::Notifications => ContentSettingTypes::NOTIFICATIONS,
        PermissionKind::Clipboard => ContentSettingTypes::CLIPBOARD_READ_WRITE,
        _ => return None,
    })
}

/// Record a permission decision for `origin` in Chromium's content
/// settings (`Some(true)` allow, `Some(false)` block, `None` ask again), so
/// pages see it through `Notification.permission`, the Permissions API and
/// Chromium's own checks. Kinds Chromium asks about through its media
/// callback (camera, microphone) need no setting.
pub fn set_permission_setting(context: &BrowsingContext, origin: &str, kind: PermissionKind, decision: Option<bool>) {
    let Some(context) = context.cef_context() else { return };
    set_permission_in(&context, origin, kind, decision);
}

/// [`set_permission_setting`] for a CEF request context.
pub(crate) fn set_permission_in(context: &RequestContext, origin: &str, kind: PermissionKind, decision: Option<bool>) {
    let Some(content_type) = content_type(kind) else { return };
    let url = CefString::from(origin);
    let value = match decision {
        Some(true) => ContentSettingValues::ALLOW,
        Some(false) => ContentSettingValues::BLOCK,
        None => ContentSettingValues::DEFAULT,
    };
    context.set_content_setting(Some(&url), Some(&url), content_type, value);
}

// ---- Third-party cookies ------------------------------------------------------------

/// Chromium's preference for blocking third-party cookies
/// (`CookieControlsMode`).
const COOKIE_CONTROLS_PREF: &str = "profile.cookie_controls_mode";
/// `CookieControlsMode::kBlockThirdParty`.
const BLOCK_THIRD_PARTY: i32 = 1;
/// `CookieControlsMode::kIncognitoOnly`, Chromium's default: allowed in
/// normal browsing.
const ALLOW_THIRD_PARTY: i32 = 2;

/// Whether this Chromium lets Mori block third-party cookies.
pub fn can_block_third_party_cookies() -> bool {
    request_context_get_global_context()
        .is_some_and(|c| c.can_set_preference(Some(&CefString::from(COOKIE_CONTROLS_PREF))) == 1)
}

/// The last third-party cookie choice (0: none made yet, else the
/// `CookieControlsMode` wanted), for contexts created after it.
static COOKIE_CHOICE: AtomicI32 = AtomicI32::new(0);

/// Block (or allow again) cookies of other sites embedded in pages of
/// `context`. Contexts created later get the same choice once Chromium has
/// set them up. Returns whether Chromium accepted the change.
pub fn set_block_third_party_cookies(context: &BrowsingContext, on: bool) -> bool {
    COOKIE_CHOICE.store(if on { BLOCK_THIRD_PARTY } else { ALLOW_THIRD_PARTY }, Ordering::Relaxed);
    let Some(context) = context.cef_context() else { return false };
    set_cookie_controls(&context, on)
}

/// A new request context is ready: give it the third-party cookie choice.
pub(crate) fn on_context_initialized(context: &RequestContext) {
    match COOKIE_CHOICE.load(Ordering::Relaxed) {
        BLOCK_THIRD_PARTY => {
            set_cookie_controls(context, true);
        }
        ALLOW_THIRD_PARTY => {
            set_cookie_controls(context, false);
        }
        _ => {}
    }
}

fn set_cookie_controls(context: &RequestContext, on: bool) -> bool {
    let name = CefString::from(COOKIE_CONTROLS_PREF);
    let wanted = if on { BLOCK_THIRD_PARTY } else { ALLOW_THIRD_PARTY };
    if context.preference(Some(&name)).is_some_and(|v| v.int() == wanted) {
        return true;
    }
    let Some(mut value) = value_create() else { return false };
    value.set_int(wanted);
    // Not empty: an empty string reaches CEF as a null pointer, and CEF
    // refuses a call without somewhere to put its error message.
    let mut error = CefString::from(" ");
    let ok = context.set_preference(
        Some(&name),
        Some(&mut value),
        Some(&mut error),
    ) == 1;
    if !ok {
        log::warn!("blocking third-party cookies failed: {error}");
    }
    ok
}

// ---- Cookies and cache ----------------------------------------------------------------

/// A stored cookie (values are never read out).
#[derive(Clone, Debug, PartialEq)]
pub struct CookieInfo {
    pub domain: String,
    pub name: String,
    pub path: String,
    /// UNIX seconds.
    pub created: f64,
    pub expires: Option<f64>,
    pub secure: bool,
    pub http_only: bool,
}

/// Microseconds between 1601-01-01 (Chromium's epoch) and 1970-01-01.
const WINDOWS_EPOCH_OFFSET_US: i64 = 11_644_473_600_000_000;

fn unix_seconds(t: &Basetime) -> f64 {
    (t.val - WINDOWS_EPOCH_OFFSET_US) as f64 / 1_000_000.0
}

type CookiePredicate = Box<dyn Fn(&CookieInfo) -> bool + Send + Sync>;
type CookiesDone = Box<dyn FnOnce(Vec<CookieInfo>) + Send>;

/// Calls `done` once the visit is over, when the last visitor reference
/// goes (CEF releases the visitor after the last cookie, or at once when
/// there are none).
struct VisitState {
    delete: Option<CookiePredicate>,
    cookies: Mutex<Vec<CookieInfo>>,
    done: Mutex<Option<CookiesDone>>,
}

impl Drop for VisitState {
    fn drop(&mut self) {
        let cookies = self.cookies.get_mut().map(std::mem::take).unwrap_or_default();
        if let Some(done) = self.done.get_mut().ok().and_then(Option::take) {
            done(cookies);
        }
    }
}

wrap_cookie_visitor! {
    pub(crate) struct CookieWalker {
        state: Arc<VisitState>,
    }

    impl CookieVisitor {
        fn visit(
            &self,
            cookie: Option<&Cookie>,
            _count: ::std::os::raw::c_int,
            _total: ::std::os::raw::c_int,
            delete_cookie: Option<&mut ::std::os::raw::c_int>,
        ) -> ::std::os::raw::c_int {
            let Some(cookie) = cookie else { return 1 };
            let info = CookieInfo {
                domain: cookie.domain.to_string(),
                name: cookie.name.to_string(),
                path: cookie.path.to_string(),
                created: unix_seconds(&cookie.creation),
                expires: (cookie.has_expires == 1).then(|| unix_seconds(&cookie.expires)),
                secure: cookie.secure == 1,
                http_only: cookie.httponly == 1,
            };
            let keep = match &self.state.delete {
                Some(delete) => {
                    let gone = delete(&info);
                    if gone && let Some(flag) = delete_cookie {
                        *flag = 1;
                    }
                    gone
                }
                None => true,
            };
            if keep && let Ok(mut cookies) = self.state.cookies.lock() {
                cookies.push(info);
            }
            1
        }
    }
}

fn walk_cookies(context: &BrowsingContext, delete: Option<CookiePredicate>, done: CookiesDone) {
    let deleting = delete.is_some();
    let state = Arc::new(VisitState { delete, cookies: Mutex::new(Vec::new()), done: Mutex::new(Some(done)) });
    let Some(manager) = context.cef_context().and_then(|c| c.cookie_manager(None)) else {
        return; // `state` drops here and reports no cookies.
    };
    let mut walker = CookieWalker::new(state);
    manager.visit_all_cookies(Some(&mut walker));
    if deleting {
        manager.flush_store(None);
    }
}

/// Every cookie stored in `context`. `done` may run on any thread.
pub fn list_cookies(context: &BrowsingContext, done: impl FnOnce(Vec<CookieInfo>) + Send + 'static) {
    walk_cookies(context, None, Box::new(done));
}

/// Delete the cookies `delete` picks; `done` gets the deleted ones and may
/// run on any thread.
pub fn delete_cookies(
    context: &BrowsingContext,
    delete: impl Fn(&CookieInfo) -> bool + Send + Sync + 'static,
    done: impl FnOnce(Vec<CookieInfo>) + Send + 'static,
) {
    walk_cookies(context, Some(Box::new(delete)), Box::new(done));
}

/// How an imported cookie is sent on cross-site requests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SameSiteMode {
    Unspecified,
    None,
    Lax,
    Strict,
}

/// A cookie to store in a browsing context. An empty `domain` is host-only;
/// a non-empty one is visible to that domain and its subdomains. `url`'s
/// host must be the cookie's host.
#[derive(Clone, Debug)]
pub struct CookieToSet {
    pub url: String,
    pub domain: String,
    pub name: String,
    pub value: String,
    pub path: String,
    /// UNIX seconds. `None` is a session cookie.
    pub expires_unix: Option<f64>,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: SameSiteMode,
}

type SetDone = Mutex<Option<Box<dyn FnOnce(usize) + Send>>>;

struct SetState {
    left: AtomicUsize,
    ok: AtomicUsize,
    done: SetDone,
}

impl SetState {
    fn settle(&self, success: bool) {
        if success {
            self.ok.fetch_add(1, Ordering::Relaxed);
        }
        if self.left.fetch_sub(1, Ordering::AcqRel) == 1 {
            let n = self.ok.load(Ordering::Relaxed);
            if let Some(done) = self.done.lock().ok().and_then(|mut d| d.take()) {
                done(n);
            }
        }
    }
}

wrap_set_cookie_callback! {
    struct CookieSet {
        state: Arc<SetState>,
    }

    impl SetCookieCallback {
        fn on_complete(&self, success: ::std::os::raw::c_int) {
            self.state.settle(success == 1);
        }
    }
}

fn basetime_from_unix(secs: f64) -> Basetime {
    Basetime { val: WINDOWS_EPOCH_OFFSET_US + (secs * 1_000_000.0) as i64 }
}

fn cef_same_site(mode: SameSiteMode) -> CookieSameSite {
    match mode {
        SameSiteMode::Unspecified => CookieSameSite::UNSPECIFIED,
        SameSiteMode::None => CookieSameSite::NO_RESTRICTION,
        SameSiteMode::Lax => CookieSameSite::LAX_MODE,
        SameSiteMode::Strict => CookieSameSite::STRICT_MODE,
    }
}

/// Store `cookies`, overwriting any with the same host, name and path.
/// `done` receives how many Chromium accepted and may run on any thread.
pub fn set_cookies(context: &BrowsingContext, cookies: Vec<CookieToSet>, done: impl FnOnce(usize) + Send + 'static) {
    let Some(manager) = context.cef_context().and_then(|c| c.cookie_manager(None)) else {
        done(0);
        return;
    };
    if cookies.is_empty() {
        done(0);
        return;
    }
    let state = Arc::new(SetState {
        left: AtomicUsize::new(cookies.len()),
        ok: AtomicUsize::new(0),
        done: Mutex::new(Some(Box::new(done))),
    });
    for cookie in &cookies {
        let (has_expires, expires) = match cookie.expires_unix {
            Some(secs) => (1, basetime_from_unix(secs)),
            None => (0, basetime_from_unix(0.0)),
        };
        let cef = Cookie {
            name: CefString::from(cookie.name.as_str()),
            value: CefString::from(cookie.value.as_str()),
            domain: CefString::from(cookie.domain.as_str()),
            path: CefString::from(if cookie.path.is_empty() { "/" } else { cookie.path.as_str() }),
            secure: cookie.secure as _,
            httponly: cookie.http_only as _,
            same_site: cef_same_site(cookie.same_site),
            priority: CookiePriority::MEDIUM,
            creation: basetime_from_unix(0.0),
            last_access: basetime_from_unix(0.0),
            has_expires,
            expires,
            ..Cookie::default()
        };
        let url = CefString::from(cookie.url.as_str());
        let mut callback = CookieSet::new(state.clone());
        if manager.set_cookie(Some(&url), Some(&cef), Some(&mut callback)) != 1 {
            state.settle(false);
        }
    }
    // Sets were queued on Chromium's IO thread; a flush queued after them
    // runs once they have landed.
    manager.flush_store(None);
}

/// A completion callback that runs once, from whichever thread finishes.
type Done = Arc<Mutex<Option<Box<dyn FnOnce() + Send>>>>;

wrap_completion_callback! {
    pub(crate) struct Completion {
        done: Done,
    }

    impl CompletionCallback {
        fn on_complete(&self) {
            if let Some(done) = self.done.lock().ok().and_then(|mut d| d.take()) {
                done();
            }
        }
    }
}

/// Empty Chromium's HTTP cache (all of it: Chromium can't clear a time
/// range). `done` may run on any thread.
pub fn clear_http_cache(context: &BrowsingContext, done: impl FnOnce() + Send + 'static) {
    let done: Done = Arc::new(Mutex::new(Some(Box::new(done))));
    match context.cef_context() {
        Some(context) => {
            let mut callback = Completion::new(done);
            context.clear_http_cache(Some(&mut callback));
        }
        None => {
            if let Some(done) = done.lock().ok().and_then(|mut d| d.take()) {
                done();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chromium_times_to_unix() {
        let t = Basetime { val: WINDOWS_EPOCH_OFFSET_US + 1_500_000 };
        assert_eq!(unix_seconds(&t), 1.5);
    }

    #[test]
    fn resource_types_use_filter_list_names() {
        assert_eq!(request_type(ResourceType::MAIN_FRAME), "document");
        assert_eq!(request_type(ResourceType::SUB_FRAME), "subdocument");
        assert_eq!(request_type(ResourceType::XHR), "xmlhttprequest");
        assert_eq!(request_type(ResourceType::FAVICON), "image");
        assert_eq!(request_type(ResourceType::SUB_RESOURCE), "other");
    }
}
