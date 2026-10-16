//! Privacy and the web platform in the browser window: the content
//! blocker, the page bridge (notifications, permission prompts, clipboard
//! reads, Picture-in-Picture requests), clearing browsing data, and site
//! data.

use std::{
    cell::RefCell,
    collections::HashSet,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use gpui_kit::*;
use mori_core::{
    content_blocker::{self, Blocker, ContentBlocker, ListCache, SiteExceptions},
    privacy::{self, BridgeMessage, ClearOptions},
    profiles,
    site_permissions::{Decision, origin_of},
    util::host_of,
    windows::WindowKind,
    workspace::TabId,
};
use mori_engine::{BrowsingContext, PermissionKind, privacy as engine};

use super::{Browser, PageOwner, ToastStyle};
use crate::notifications::{Notifier, WebNotification};

/// How often Mori checks whether the filter lists are due for a refresh.
const LIST_CHECK_INTERVAL: Duration = Duration::from_secs(6 * 3600);
/// Second pass of generic cosmetic filtering, for content pages add late.
const LATE_COSMETICS_DELAY: Duration = Duration::from_secs(3);

/// Page functions of the bridge script (see `mori_core::privacy`).
const REPLY_FUNCTION: &str = "__moriReply";
const EVENT_FUNCTION: &str = "__moriEvent";

/// State of the downloaded filter lists, for the settings page.
#[derive(Clone, Debug, Default)]
pub struct ListsStatus {
    /// Rules in the engine in use; `None` while the built-in list is used.
    pub rules: Option<usize>,
    /// When the lists were last downloaded (UNIX seconds).
    pub updated_at: Option<f64>,
    pub updating: bool,
    pub last_error: Option<String>,
}

/// Process-wide privacy machinery shared by every window.
pub struct PrivacyRuntime {
    pub blocker: Arc<ContentBlocker>,
    pub status: Arc<Mutex<ListsStatus>>,
    data_dir: PathBuf,
}

impl Global for PrivacyRuntime {}

/// The window's privacy state.
#[derive(Default)]
pub struct PrivacyState {
    /// Sites the content blocker is off for.
    pub exceptions: SiteExceptions,
    /// Choices in "Clear browsing data".
    pub clear: ClearOptions,
    /// Sites with cookies and how many, once loaded for the settings page.
    pub cookie_sites: Option<Vec<(String, usize)>>,
    pub loading_cookies: bool,
    /// "Remove All" cookies was clicked once and waits for confirmation.
    pub confirm_remove_cookies: bool,
}

impl PrivacyState {
    /// The state of a new window, with the blocker exceptions every window
    /// shares (see `windows::Seed`).
    pub fn new(exceptions: SiteExceptions) -> PrivacyState {
        PrivacyState { exceptions, ..Default::default() }
    }
}

thread_local! {
    /// Profile contexts (by directory name) that Mori's permission and
    /// cookie choices have been applied to this run.
    static PREPARED_PROFILES: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

/// The permission kind a remembered permission name stands for, when
/// Chromium keeps a content setting for it.
fn setting_kind(key: &str) -> Option<PermissionKind> {
    Some(match key {
        "geolocation" => PermissionKind::Geolocation,
        "notifications" => PermissionKind::Notifications,
        "clipboard" => PermissionKind::Clipboard,
        _ => return None,
    })
}

impl Browser {
    /// Start the content blocker (once per process) and apply this window's
    /// privacy settings to Chromium.
    pub(super) fn start_privacy(&mut self, cx: &mut Context<Self>) {
        if !cx.has_global::<PrivacyRuntime>() {
            let runtime = PrivacyRuntime {
                blocker: Arc::new(ContentBlocker::default()),
                status: Arc::new(Mutex::new(ListsStatus::default())),
                data_dir: mori_core::paths::data_dir(),
            };
            let blocker = runtime.blocker.clone();
            engine::set_request_filter(Some(Arc::new(move |r| {
                blocker.should_block(r.url, r.page_url, r.request_type)
            })));
            load_filter_lists(runtime.blocker.clone(), runtime.status.clone(), runtime.data_dir.clone(), false);
            cx.set_global(runtime);
            cx.spawn(async move |_, cx| {
                loop {
                    cx.background_executor().timer(LIST_CHECK_INTERVAL).await;
                    let Some((blocker, status, dir)) = cx.update(|cx| {
                        cx.try_global::<PrivacyRuntime>()
                            .map(|r| (r.blocker.clone(), r.status.clone(), r.data_dir.clone()))
                    }) else {
                        break;
                    };
                    load_filter_lists(blocker, status, dir, true);
                }
            })
            .detach();
        }
        // Picture-in-Picture goes with its browser window.
        let release = cx.on_release(|this, cx| {
            if let Some(pip) = this.pip.take() {
                pip.close_window(cx);
            }
        });
        self.push_subscription(release);
        let blocker = cx.global::<PrivacyRuntime>().blocker.clone();
        blocker.set_enabled(self.settings.content_blocker);
        blocker.set_exceptions(&self.privacy.exceptions);
        self.apply_tracking_settings();
        for context in self.privacy_contexts(false) {
            self.apply_permission_settings(&context);
            if let Some(dir) = context.dir_name() {
                PREPARED_PROFILES.with(|p| p.borrow_mut().insert(dir.to_string()));
            }
        }
    }

    /// The browsing contexts this window's privacy choices go to: a private
    /// window's own context, or else the default context and the profiles'
    /// (those loaded this run; `all_profiles` opens every profile).
    pub(crate) fn privacy_contexts(&self, all_profiles: bool) -> Vec<BrowsingContext> {
        if self.kind == WindowKind::Private {
            return self.private_context.iter().cloned().collect();
        }
        let mut out = vec![BrowsingContext::shared()];
        if all_profiles {
            for profile in crate::windows::profiles() {
                out.extend(BrowsingContext::persistent(&profiles::dir_name(profile.id)));
            }
        }
        for context in BrowsingContext::loaded_persistent() {
            if !out.iter().any(|c| c.same_as(&context)) {
                out.push(context);
            }
        }
        out
    }

    /// Write the remembered notification, location and clipboard decisions
    /// into `context`'s content settings.
    fn apply_permission_settings(&self, context: &BrowsingContext) {
        for (origin, decisions) in self.site_permissions.sites() {
            for (key, decision) in decisions {
                if let Some(kind) = setting_kind(&key) {
                    engine::set_permission_setting(context, origin, kind, Some(decision == Decision::Allow));
                }
            }
        }
    }

    /// A profile's context was opened: give it Mori's permission choices
    /// once per run. (The engine gives new contexts the third-party cookie
    /// choice itself, once Chromium has set them up.)
    pub(crate) fn prepare_profile_context(&self, context: &BrowsingContext) {
        let Some(dir) = context.dir_name() else { return };
        if !PREPARED_PROFILES.with(|p| p.borrow_mut().insert(dir.to_string())) {
            return;
        }
        self.apply_permission_settings(context);
    }

    /// Push Do Not Track and third-party cookie choices to Chromium, and
    /// (re)install the page bridge, which carries the DNT flag.
    pub fn apply_tracking_settings(&self) {
        let dnt = self.settings.do_not_track;
        engine::set_do_not_track(dnt);
        mori_engine::set_page_bridge(mori_engine::PageBridge {
            console_marker: privacy::BRIDGE_MARKER.into(),
            script: privacy::page_bridge_script(dnt),
            token_placeholder: privacy::BRIDGE_TOKEN_PLACEHOLDER.into(),
            reply_function: REPLY_FUNCTION.into(),
            event_function: EVENT_FUNCTION.into(),
        });
        if engine::can_block_third_party_cookies() {
            for context in self.privacy_contexts(false) {
                engine::set_block_third_party_cookies(&context, self.settings.block_third_party_cookies);
            }
        }
    }

    pub fn content_blocker(&self, cx: &App) -> Option<Arc<ContentBlocker>> {
        cx.try_global::<PrivacyRuntime>().map(|r| r.blocker.clone())
    }

    pub fn lists_status(&self, cx: &App) -> ListsStatus {
        cx.try_global::<PrivacyRuntime>()
            .and_then(|r| r.status.lock().ok().map(|s| s.clone()))
            .unwrap_or_default()
    }

    /// Download fresh filter lists now (settings button).
    pub fn update_filter_lists(&mut self, cx: &mut Context<Self>) {
        let Some(r) = cx.try_global::<PrivacyRuntime>() else { return };
        let (blocker, status, dir) = (r.blocker.clone(), r.status.clone(), r.data_dir.clone());
        // Forget the last download time so every list is due.
        let cache = ListCache::new(&dir);
        let mut meta = cache.meta();
        meta.fetched.clear();
        meta.last_failure = None;
        let _ = cache.save_meta(&meta);
        load_filter_lists(blocker, status.clone(), dir, true);
        self.watch_list_update(status, cx);
        cx.notify();
    }

    /// Redraw while a list update runs, so the settings page shows its end.
    fn watch_list_update(&mut self, status: Arc<Mutex<ListsStatus>>, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(500)).await;
                let updating = status.lock().map(|s| s.updating).unwrap_or(false);
                if this.update(cx, |_, cx| cx.notify()).is_err() || !updating {
                    break;
                }
            }
        })
        .detach();
    }

    pub fn set_content_blocker_enabled(&mut self, on: bool, cx: &mut Context<Self>) {
        self.settings.content_blocker = on;
        self.sync_content_blocker(cx);
        self.schedule_save(cx);
        cx.notify();
    }

    /// Switch the running blocker on or off to match the setting.
    pub fn sync_content_blocker(&self, cx: &App) {
        if let Some(b) = self.content_blocker(cx) {
            b.set_enabled(self.settings.content_blocker);
        }
    }

    /// Whether blocking is on for the page at `url` (switched on, and the
    /// site is no exception).
    pub fn blocking_active_for(&self, url: &str) -> bool {
        self.settings.content_blocker && !self.privacy.exceptions.is_exempt(url)
    }

    /// Turn the content blocker off or back on for the site of the tab's
    /// page, then reload it so the change shows.
    pub fn toggle_site_blocking(&mut self, tab: TabId, cx: &mut Context<Self>) {
        let Some(url) = self.ws.tab(tab).map(|t| t.url.clone()) else { return };
        let exempt = !self.privacy.exceptions.is_exempt(&url);
        if !self.privacy.exceptions.set_exempt(&url, exempt) {
            return;
        }
        if let Some(b) = self.content_blocker(cx) {
            b.set_exceptions(&self.privacy.exceptions);
        }
        if let Some(live) = self.live.get(&tab) {
            live.view.reload(false);
        }
        let host = content_blocker::site_key(&url).unwrap_or_default();
        let text = if exempt { format!("Blocking off for {host}") } else { format!("Blocking on for {host}") };
        self.toast(text, ToastStyle::Info, None, cx);
        self.schedule_save(cx);
        cx.notify();
    }

    pub fn remove_blocking_exception(&mut self, site: &str, cx: &mut Context<Self>) {
        if self.privacy.exceptions.remove_site(site) {
            if let Some(b) = self.content_blocker(cx) {
                b.set_exceptions(&self.privacy.exceptions);
            }
            self.schedule_save(cx);
            cx.notify();
        }
    }

    // ---- Cosmetic filtering ------------------------------------------------------

    /// Hide the elements the page's host-specific rules name, as soon as
    /// its document exists.
    pub(super) fn apply_site_cosmetics(&self, id: TabId, cx: &App) {
        let (Some(live), Some(tab)) = (self.live.get(&id), self.ws.tab(id)) else { return };
        let Some(cosmetics) = self.content_blocker(cx).and_then(|b| b.cosmetics(&tab.url)) else { return };
        if !cosmetics.css.is_empty() {
            live.view.execute_js(&content_blocker::inject_css_script("site", &cosmetics.css));
        }
    }

    /// Apply generic cosmetic rules for the classes and ids on the loaded
    /// page, now and once more a little later.
    pub(super) fn apply_generic_cosmetics(&self, id: TabId, cx: &mut Context<Self>) {
        // The host rules again: injected on the address change, they may
        // have reached the document being replaced. (Same style element, so
        // this is a no-op when they did land.)
        self.apply_site_cosmetics(id, cx);
        self.collect_and_hide(id, cx);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(LATE_COSMETICS_DELAY).await;
            let _ = this.update(cx, |this, cx| this.collect_and_hide(id, cx));
        })
        .detach();
    }

    fn collect_and_hide(&self, id: TabId, cx: &App) {
        let (Some(live), Some(tab)) = (self.live.get(&id), self.ws.tab(id)) else { return };
        let Some(blocker) = self.content_blocker(cx) else { return };
        let Some(cosmetics) = blocker.cosmetics(&tab.url) else { return };
        if cosmetics.generichide {
            return;
        }
        let view = live.view.clone();
        let target = view.clone();
        view.evaluate(content_blocker::COLLECT_CLASSES_SCRIPT, move |result| {
            #[derive(serde::Deserialize, Default)]
            #[serde(default)]
            struct Found {
                classes: Vec<String>,
                ids: Vec<String>,
            }
            let mut found: Found = result.ok().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
            // The list comes from page script, which can return anything:
            // keep to the script's own cap.
            found.classes.truncate(content_blocker::MAX_COLLECTED_NAMES);
            found.ids.truncate(content_blocker::MAX_COLLECTED_NAMES);
            let Some(engine) = blocker.blocker() else { return };
            let css = engine.generic_css(&found.classes, &found.ids, &cosmetics.exceptions);
            if !css.is_empty() {
                target.execute_js(&content_blocker::inject_css_script("generic", &css));
            }
        });
    }

    // ---- Page bridge ---------------------------------------------------------------

    /// A message from the page bridge script of `owner`'s page.
    pub(super) fn on_page_message(&mut self, owner: PageOwner, value: serde_json::Value, cx: &mut Context<Self>) {
        let Some(message) = BridgeMessage::parse(&value) else { return };
        let PageOwner::Tab(tab) = owner else {
            // Peek and popups are transient: they get no notifications,
            // clipboard reads or Picture-in-Picture.
            if let (BridgeMessage::Pip { rid: Some(rid), .. } | BridgeMessage::ClipRead { rid }, Some(view)) =
                (&message, self.owner_view(owner))
            {
                view.bridge_reply(*rid, &serde_json::Value::Null);
            }
            return;
        };
        let Some(url) = self.ws.tab(tab).map(|t| t.url.clone()) else { return };
        match message {
            BridgeMessage::Notify { nid, title, body, tag, silent } => {
                if self.site_permissions.get(&url, "notifications") != Some(Decision::Allow) {
                    return;
                }
                let (title, body) = privacy::notification_text(&title, &body);
                let icon = host_of(&url).and_then(|h| self.favicons.path(&h));
                Notifier::show(
                    WebNotification {
                        browser: cx.entity().downgrade(),
                        tab,
                        nid,
                        source: privacy::notification_source(&url),
                        title,
                        body,
                        tag: Some(tag),
                        icon,
                        silent,
                    },
                    cx,
                );
            }
            BridgeMessage::NotifyClose { nid } => Notifier::close(tab, nid, cx),
            BridgeMessage::ClipRead { rid } => {
                // Only the page on screen may read the clipboard; the bridge
                // script also requires its document to have focus, as Chrome
                // does.
                let allowed = self.site_permissions.get(&url, "clipboard") == Some(Decision::Allow)
                    && self.visible_tabs().contains(&Some(tab));
                let text = allowed
                    .then(|| cx.read_from_clipboard().and_then(|c| c.text()))
                    .flatten()
                    .map(serde_json::Value::String)
                    .unwrap_or(serde_json::Value::Null);
                if let Some(live) = self.live.get(&tab) {
                    live.view.bridge_reply(rid, &text);
                }
            }
            BridgeMessage::Pip { rid, open } => {
                // A page asks from a user gesture, so only the page on
                // screen can (or the one already in Picture-in-Picture).
                let ok = if open {
                    (self.pip_tab() == Some(tab) || self.visible_tabs().contains(&Some(tab)))
                        && self.open_pip(tab, true, cx)
                } else {
                    if self.pip_tab() == Some(tab) {
                        self.close_pip(false, cx);
                    }
                    true
                };
                if let (Some(rid), Some(live)) = (rid, self.live.get(&tab)) {
                    live.view.bridge_reply(rid, &serde_json::Value::Bool(ok));
                }
            }
        }
    }

    /// The desktop says a notification of `tab` was clicked (or closed).
    pub fn on_notification_answer(&mut self, tab: TabId, nid: u64, clicked: bool, window: &mut Window, cx: &mut Context<Self>) {
        if clicked && self.ws.tab(tab).is_some() {
            self.select_tab(tab, window, cx);
        }
        if let Some(live) = self.live.get(&tab) {
            let t = if clicked { "notification-click" } else { "notification-close" };
            live.view.bridge_event(&serde_json::json!({ "t": t, "nid": nid }), clicked);
        }
    }

    pub fn window_handle(&self) -> AnyWindowHandle {
        self.window_handle
    }

    /// Chromium keeps notification, location and clipboard decisions as
    /// content settings too; keep them in step with Mori's after the user
    /// changed or reset one for `url`.
    pub fn sync_permission_setting(&self, url: &str, key: &str) {
        let (Some(kind), Some(origin)) = (setting_kind(key), origin_of(url)) else { return };
        let decision = self.site_permissions.get(url, key).map(|d| d == Decision::Allow);
        for context in self.privacy_contexts(false) {
            engine::set_permission_setting(&context, &origin, kind, decision);
        }
    }

    /// Forget every remembered permission of `url`'s site.
    pub fn reset_site_permissions(&mut self, url: &str, cx: &mut Context<Self>) {
        let keys: Vec<String> = self.site_permissions.for_site(url).into_iter().map(|(k, _)| k).collect();
        self.site_permissions.forget_site(url);
        for key in keys {
            self.sync_permission_setting(url, &key);
        }
        self.schedule_save(cx);
        cx.notify();
    }

    /// Forget one remembered permission (settings' site list).
    pub fn revoke_permission(&mut self, url: &str, key: &str, cx: &mut Context<Self>) {
        if self.site_permissions.forget(url, key) {
            self.sync_permission_setting(url, key);
            self.schedule_save(cx);
            cx.notify();
        }
    }

    /// Flip a remembered permission between allowed and blocked.
    pub fn flip_permission(&mut self, url: &str, key: &str, cx: &mut Context<Self>) {
        let next = match self.site_permissions.get(url, key) {
            Some(Decision::Allow) => Decision::Block,
            _ => Decision::Allow,
        };
        self.site_permissions.set(url, key, next);
        self.sync_permission_setting(url, key);
        self.schedule_save(cx);
        cx.notify();
    }

    // ---- Browsing data --------------------------------------------------------------

    /// Clear what `opts` picks, for its time range.
    pub fn clear_browsing_data(&mut self, opts: ClearOptions, cx: &mut Context<Self>) {
        if !opts.any() {
            return;
        }
        let since = opts.range.since(mori_core::paths::now());
        // Origins whose storage goes with the cookies: pages visited in the
        // range (gathered before history itself may be cleared).
        let visited = privacy::origins_of(self.history.urls_since(since));
        if opts.history {
            match since {
                Some(t) => {
                    self.history.remove_since(t);
                }
                None => self.history.clear(),
            }
        }
        if opts.downloads {
            self.downloads.clear_finished_since(since);
        }
        if opts.cache {
            for context in self.privacy_contexts(true) {
                engine::clear_http_cache(&context, || {});
            }
        }
        if opts.cookies {
            self.privacy.cookie_sites = None;
            let delete = move |c: &engine::CookieInfo| since.is_none_or(|t| c.created >= t);
            self.delete_cookies_then_storage(delete, visited, since.is_none(), cx);
        }
        self.toast(opts.summary(), ToastStyle::Success, None, cx);
        self.schedule_save(cx);
        cx.notify();
    }

    /// Delete cookies `delete` picks, then the stored data (local storage,
    /// IndexedDB, caches, service workers) of `origins` and of the sites the
    /// deleted cookies belonged to.
    ///
    /// This happens in every browsing context the window's choices go to:
    /// the default context and each profile's, or a private window's own.
    fn delete_cookies_then_storage(
        &mut self,
        delete: impl Fn(&engine::CookieInfo) -> bool + Send + Sync + 'static,
        origins: std::collections::BTreeSet<String>,
        everything: bool,
        cx: &mut Context<Self>,
    ) {
        let delete = Arc::new(delete);
        let mut pending = Vec::new();
        for context in self.privacy_contexts(true) {
            let (tx, rx) = futures::channel::oneshot::channel();
            let delete = delete.clone();
            engine::delete_cookies(&context, move |c| delete(c), move |deleted| {
                let _ = tx.send(deleted);
            });
            pending.push((context, rx));
        }
        cx.spawn(async move |this, cx| {
            let mut done = Vec::new();
            for (context, rx) in pending {
                done.push((context, rx.await.unwrap_or_default()));
            }
            let _ = this.update(cx, |this, cx| {
                for (context, deleted) in done {
                    let mut origins = origins.clone();
                    for cookie in &deleted {
                        let host = cookie.domain.trim_start_matches('.');
                        origins.insert(format!("https://{host}"));
                        origins.insert(format!("http://{host}"));
                    }
                    this.clear_site_storage(&context, origins, everything);
                }
                if this.privacy.cookie_sites.is_some() || this.privacy.loading_cookies {
                    this.load_cookie_sites(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Clear the stored data of `origins` in `context` through the DevTools
    /// session of one of its pages (the call reaches the whole context).
    /// `with_cookies` also clears their cookies.
    fn clear_site_storage(
        &self,
        context: &BrowsingContext,
        origins: std::collections::BTreeSet<String>,
        with_cookies: bool,
    ) {
        let view = self
            .live
            .iter()
            .filter(|(_, l)| l.view.is_created())
            .find(|(tab, _)| crate::windows::context_for_tab(self, **tab).same_as(context))
            .map(|(_, l)| l.view.clone());
        let Some(view) = view else {
            // Cookies are gone either way; storage waits for a page.
            log::info!("no page in {context:?} to clear site storage through");
            return;
        };
        let types = if with_cookies {
            "all"
        } else {
            "local_storage,indexeddb,cache_storage,service_workers,websql,file_systems,shared_storage,storage_buckets"
        };
        for origin in origins {
            view.devtools_call(
                "Storage.clearDataForOrigin",
                serde_json::json!({ "origin": origin, "storageTypes": types }),
                move |r| {
                    if let Err(e) = r {
                        log::debug!("clearing storage of {origin}: {e}");
                    }
                },
            );
        }
    }

    /// Delete the cookies and stored data of the site of `url` (site info).
    pub fn clear_site_data(&mut self, url: &str, cx: &mut Context<Self>) {
        let Some(host) = host_of(url) else { return };
        let site = privacy::site_of_host(&host);
        let origins = [format!("https://{host}"), format!("http://{host}")].into_iter().collect();
        let site_for_cookies = site.clone();
        self.delete_cookies_then_storage(
            move |c| privacy::cookie_in_site(&c.domain, &site_for_cookies),
            origins,
            false,
            cx,
        );
        self.toast(format!("Cleared cookies and site data for {site}"), ToastStyle::Success, None, cx);
    }

    /// Delete every cookie of `site` (settings' cookie list).
    pub fn remove_cookie_site(&mut self, site: &str, cx: &mut Context<Self>) {
        let site = site.to_string();
        let origins = [format!("https://{site}"), format!("https://www.{site}")].into_iter().collect();
        self.delete_cookies_then_storage(move |c| privacy::cookie_in_site(&c.domain, &site), origins, false, cx);
    }

    /// Read which sites have cookies, for the settings page.
    pub fn load_cookie_sites(&mut self, cx: &mut Context<Self>) {
        self.privacy.loading_cookies = true;
        let mut pending = Vec::new();
        for context in self.privacy_contexts(true) {
            let (tx, rx) = futures::channel::oneshot::channel();
            engine::list_cookies(&context, move |cookies| {
                let _ = tx.send(cookies);
            });
            pending.push(rx);
        }
        cx.spawn(async move |this, cx| {
            let mut cookies = Vec::new();
            for rx in pending {
                cookies.extend(rx.await.unwrap_or_default());
            }
            let _ = this.update(cx, |this, cx| {
                this.privacy.cookie_sites = Some(privacy::cookie_sites(cookies.iter().map(|c| c.domain.as_str())));
                this.privacy.loading_cookies = false;
                cx.notify();
            });
        })
        .detach();
    }
}

/// Load the compiled filter lists (or build them, or fall back to the
/// built-in list), then download any list that is due and rebuild. Runs on
/// its own thread: downloads and compiling take seconds.
fn load_filter_lists(blocker: Arc<ContentBlocker>, status: Arc<Mutex<ListsStatus>>, dir: PathBuf, update_only: bool) {
    let set_status = {
        let status = status.clone();
        move |f: &dyn Fn(&mut ListsStatus)| {
            if let Ok(mut s) = status.lock() {
                f(&mut s);
            }
        }
    };
    if let Ok(mut s) = status.lock() {
        if s.updating {
            return;
        }
        s.updating = true;
    }
    let spawned = std::thread::Builder::new().name("mori-filter-lists".into()).spawn(move || {
        let cache = ListCache::new(&dir);
        let mut meta = cache.meta();
        if !update_only || blocker.blocker().is_none() {
            let (engine, from_lists) = match cache.load_engine() {
                Some(engine) => (engine, true),
                None => match cache.read_lists() {
                    Some(lists) => {
                        let engine = Blocker::from_lists(&lists);
                        let _ = cache.save_engine(&engine);
                        meta.rule_count = content_blocker::count_network_rules(&lists);
                        (engine, true)
                    }
                    None => (Blocker::bundled(), false),
                },
            };
            blocker.set_blocker(Arc::new(engine));
            let rules = from_lists.then_some(meta.rule_count);
            let updated = meta.fetched.values().copied().reduce(f64::min);
            set_status(&|s| {
                s.rules = rules;
                s.updated_at = updated;
            });
        }
        let now = mori_core::paths::now();
        let due = content_blocker::lists_due(&meta, now);
        let mut failure = None;
        for list in &due {
            match fetch_list(list.url).and_then(|text| cache.store_list(list, &text)) {
                Ok(()) => {
                    meta.fetched.insert(list.id.to_string(), now);
                }
                Err(e) => {
                    log::warn!("updating {}: {e:#}", list.name);
                    failure = Some(format!("{e:#}"));
                    break;
                }
            }
        }
        meta.last_failure = failure.as_ref().map(|_| now);
        if failure.is_none()
            && !due.is_empty()
            && let Some(lists) = cache.read_lists()
        {
            let engine = Blocker::from_lists(&lists);
            if let Err(e) = cache.save_engine(&engine) {
                log::warn!("saving the content blocker: {e:#}");
            }
            meta.rule_count = content_blocker::count_network_rules(&lists);
            blocker.set_blocker(Arc::new(engine));
            log::info!("content blocker updated: {} rules", meta.rule_count);
        }
        let _ = cache.save_meta(&meta);
        let rules = cache.read_lists().map(|_| meta.rule_count);
        let updated = meta.fetched.values().copied().reduce(f64::min);
        set_status(&|s| {
            s.updating = false;
            s.last_error = failure.clone();
            if rules.is_some() {
                s.rules = rules;
                s.updated_at = updated;
            }
        });
    });
    if spawned.is_err()
        && let Ok(mut s) = status.lock()
    {
        s.updating = false;
    }
}

/// Download a filter list. Blocking.
fn fetch_list(url: &str) -> anyhow::Result<String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(60)))
        .user_agent(format!("Mori/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    let mut response = agent.get(url).call()?;
    Ok(response.body_mut().with_config().limit(16 * 1024 * 1024).read_to_string()?)
}
