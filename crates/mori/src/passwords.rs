//! The password manager: offering to save logins people submit, filling
//! saved logins back in, suggesting strong passwords, and the Passwords
//! settings page.
//!
//! Chromium's own password manager doesn't run in CEF's windowless mode,
//! so Mori does it itself:
//! - A form agent (`mori_core::passwords::scripts`) runs in an isolated
//!   world of every tab's main frame and reports login forms, focus on
//!   their fields and submitted logins through a DevTools binding. Page
//!   script can't see the agent or forge its reports.
//! - Secrets go to the system keyring ([`vault`]); `passwords.json` only
//!   has origins, usernames and timestamps. Without a keyring nothing is
//!   saved, and the settings page says why.
//! - Saved logins are filled only into the main frame of a page whose
//!   origin matches exactly, or, after the user confirms, a related one
//!   (`mori_core::passwords::match_origin`).

mod popover;
pub mod settings;
mod vault;

use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

use gpui_kit::*;
use mori_core::{
    passwords::{
        OriginMatch, PasswordStore, SaveTarget, Secret, generate_password, match_origin,
        report::{AGENT_BINDING, AGENT_WORLD, FieldKind, FieldRect, FormReport, parse_report},
        scripts::{FORM_AGENT_SCRIPT, FillMode, fill_script},
    },
    persist::Saver,
    workspace::TabId,
};
use mori_engine::{ViewEvent, ViewId, WebView};
use uuid::Uuid;

pub use popover::PasswordsUi;
pub use settings::PasswordsPage;
pub use vault::VaultStatus;

use crate::browser::{Browser, ToastStyle};

/// How long after a submission a navigation (or an SPA's form going away)
/// still counts as its result. Longer, and leaving a site after a sign-in
/// that went nowhere would offer to save what was typed there.
const SUBMIT_WINDOW: Duration = Duration::from_secs(20);
/// A save offer survives page loads this soon after it was made (sign-ins
/// that bounce through a redirect page); later ones drop it.
const OFFER_GRACE: Duration = Duration::from_secs(5);
/// How long the username of a two-step sign-in is remembered for its
/// password step.
const USERNAME_STEP_WINDOW: Duration = Duration::from_secs(600);
/// After a submission navigated, how long to wait for the new page's forms
/// before deciding whether the sign-in worked (the login form is gone).
const RESULT_DELAY: Duration = Duration::from_millis(1200);

/// The focused login field of a page.
#[derive(Clone, Copy, Debug)]
pub struct FieldFocus {
    pub kind: FieldKind,
    pub rect: FieldRect,
}

/// A submitted login waiting to see whether the sign-in worked.
struct Submission {
    origin: String,
    username: String,
    password: Secret,
    at: Instant,
    navigated: bool,
}

/// What the save popover offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OfferKind {
    Save,
    Update(Uuid),
}

/// A login the user just signed in with that Mori doesn't have (or has
/// with another password).
pub struct SaveOffer {
    pub origin: String,
    pub username: String,
    pub password: Secret,
    pub kind: OfferKind,
    /// Bumped per offer, so the popover's username field is made afresh.
    pub serial: u64,
    at: Instant,
}

/// Which popover hangs off the URL pill's key button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PillPopover {
    Offer,
    Logins,
}

/// Login state of one page (view).
#[derive(Default)]
pub struct PageLogins {
    /// The page's origin, as the agent last reported it.
    pub origin: Option<String>,
    pub login_form: bool,
    pub new_password_form: bool,
    pub focus: Option<FieldFocus>,
    /// The suggestions dropdown under the focused field is open.
    pub dropdown: bool,
    /// A related-site login waiting for the user to confirm filling it.
    pub confirm: Option<Uuid>,
    /// The strong password suggested for this page's new-password fields.
    pub suggestion: Option<Secret>,
    pub offer: Option<SaveOffer>,
    pub popover: Option<PillPopover>,
    pending: Option<Submission>,
    last_username: Option<(String, String, Instant)>,
}

/// App-wide password state: the store, the keyring, and each page's
/// login state.
pub struct Passwords {
    pub store: PasswordStore,
    pub status: VaultStatus,
    vault: vault::Vault,
    saver: Saver,
    agents: HashSet<ViewId>,
    pages: HashMap<ViewId, PageLogins>,
    warned_unavailable: bool,
    /// A keyring check is under way.
    probing: bool,
    next_serial: u64,
}

impl Global for Passwords {}

impl Passwords {
    /// The app's password state, loaded on first use.
    pub fn get(cx: &mut App) -> &mut Passwords {
        if !cx.has_global::<Passwords>() {
            let dir = mori_core::paths::data_dir();
            cx.set_global(Passwords {
                store: PasswordStore::load(&dir),
                status: VaultStatus::Unknown,
                vault: vault::Vault::new(),
                saver: Saver::new(&dir),
                agents: HashSet::new(),
                pages: HashMap::new(),
                warned_unavailable: false,
                probing: false,
                next_serial: 1,
            });
            cx.on_app_quit(|cx| {
                if cx.has_global::<Passwords>() {
                    let p = cx.global_mut::<Passwords>();
                    p.save();
                    p.saver.flush();
                }
                async {}
            })
            .detach();
        }
        cx.global_mut::<Passwords>()
    }

    /// Queue `passwords.json` for writing.
    pub fn save(&mut self) {
        if let Err(e) = self.saver.stage(mori_core::passwords::PASSWORDS_FILE, &self.store) {
            log::error!("saving passwords: {e:#}");
        }
    }

    pub fn page(&self, view: ViewId) -> Option<&PageLogins> {
        self.pages.get(&view)
    }

    fn page_mut(&mut self, view: ViewId) -> &mut PageLogins {
        self.pages.entry(view).or_default()
    }

    /// Run `f` on the keyring thread; `done` gets the result on the UI
    /// thread, with the browser window that asked.
    pub fn vault_call<T: Send + 'static>(
        cx: &mut Context<Browser>,
        f: impl FnOnce(&secret_service::blocking::SecretService) -> Result<T, String> + Send + 'static,
        done: impl FnOnce(&mut Browser, Result<T, String>, &mut Context<Browser>) + 'static,
    ) {
        let rx = Passwords::get(cx).vault.run(f);
        cx.spawn(async move |this, cx| {
            let result = rx.await.unwrap_or_else(|_| Err("The keyring stopped responding.".into()));
            let _ = this.update(cx, |this, cx| done(this, result, cx));
        })
        .detach();
    }

    /// Check (again) whether a keyring is reachable.
    pub fn probe(cx: &mut Context<Browser>) {
        Passwords::vault_call(cx, vault::probe, |_, result, cx| {
            let status = match result {
                Ok(()) => VaultStatus::Ready,
                Err(reason) => {
                    log::warn!("passwords: no keyring: {reason}");
                    VaultStatus::Unavailable(reason)
                }
            };
            let p = Passwords::get(cx);
            p.status = status;
            p.probing = false;
            cx.notify();
        });
    }
}

/// Whether a page event matters to the password manager (most are paints).
fn concerns_logins(ev: &ViewEvent) -> bool {
    matches!(
        ev,
        ViewEvent::LoadEnd { .. }
            | ViewEvent::LoadingStateChanged { .. }
            | ViewEvent::AddressChanged(_)
            | ViewEvent::BindingCalled { .. }
    )
}

/// The view of a live tab.
fn tab_view(b: &Browser, tab: TabId) -> Option<WebView> {
    b.live.get(&tab).map(|l| l.view.clone())
}

/// Install the form agent in `view`: the binding it reports through
/// (only in Mori's isolated world) and the script itself, for the current
/// document and every later one. `Runtime.enable` is needed for binding
/// calls to be reported.
fn install_agent(view: &WebView) {
    let log_error = |what: &'static str| {
        move |r: Result<serde_json::Value, String>| {
            if let Err(e) = r {
                log::warn!("passwords: {what} failed: {e}");
            }
        }
    };
    view.devtools_call("Runtime.enable", serde_json::json!({}), log_error("Runtime.enable"));
    view.devtools_call("Page.enable", serde_json::json!({}), log_error("Page.enable"));
    view.devtools_call(
        "Runtime.addBinding",
        serde_json::json!({ "name": AGENT_BINDING, "executionContextName": AGENT_WORLD }),
        log_error("Runtime.addBinding"),
    );
    view.devtools_call(
        "Page.addScriptToEvaluateOnNewDocument",
        serde_json::json!({ "source": FORM_AGENT_SCRIPT, "worldName": AGENT_WORLD, "runImmediately": true }),
        log_error("installing the form agent"),
    );
}

/// Evaluate `script` in Mori's isolated world of `view`'s main frame;
/// `done` gets the script's value.
fn evaluate_in_agent_world(view: &WebView, script: Secret, done: impl FnOnce(Result<serde_json::Value, String>) + 'static) {
    let v = view.clone();
    view.devtools_call("Page.getFrameTree", serde_json::json!({}), move |tree| {
        let Some(frame) = tree.ok().and_then(|t| t.pointer("/frameTree/frame/id").and_then(|id| id.as_str()).map(str::to_string)) else {
            done(Err("the page has no main frame".into()));
            return;
        };
        let v2 = v.clone();
        v.devtools_call(
            "Page.createIsolatedWorld",
            serde_json::json!({ "frameId": frame, "worldName": AGENT_WORLD }),
            move |world| {
                let Some(context) = world.ok().and_then(|w| w.get("executionContextId").and_then(|c| c.as_i64())) else {
                    done(Err("no isolated world".into()));
                    return;
                };
                let params = serde_json::json!({
                    "expression": script.expose(),
                    "contextId": context,
                    "returnByValue": true,
                });
                drop(script);
                v2.devtools_call("Runtime.evaluate", params, move |result| {
                    done(result.map(|r| r.pointer("/result/value").cloned().unwrap_or_default()))
                });
            },
        );
    });
}

impl Browser {
    /// Follow a tab's page events for logins (called for every event of a
    /// tab, before the browser handles it).
    pub fn passwords_on_view_event(&mut self, tab: TabId, ev: &mut ViewEvent, cx: &mut Context<Self>) {
        if !concerns_logins(ev) {
            return;
        }
        if let Some(view) = tab_view(self, tab) {
            self.passwords_on_page_event(view, ev, cx);
        }
    }

    /// The same for the floating `window.open` popup (sign-in windows).
    pub fn passwords_on_popup_event(&mut self, ev: &mut ViewEvent, cx: &mut Context<Self>) {
        if !concerns_logins(ev) {
            return;
        }
        if let Some(view) = self.popup.as_ref().map(|p| p.view.clone()) {
            self.passwords_on_page_event(view, ev, cx);
        }
    }

    /// A sign-in popup closed (usually right after signing in): what it
    /// was about to offer moves to the page that opened it.
    pub fn passwords_popup_closed(&mut self, popup: ViewId, opener: TabId, cx: &mut Context<Self>) {
        let Some(opener) = tab_view(self, opener).map(|v| v.id()) else { return };
        let p = Passwords::get(cx);
        let Some(mut page) = p.pages.remove(&popup) else { return };
        let target = p.page_mut(opener);
        if page.offer.is_some() {
            target.offer = page.offer.take();
            target.popover = Some(PillPopover::Offer);
        } else if let Some(pending) = page.pending.take() {
            // The popup closing itself is the sign-in finishing.
            self.passwords_offer(opener, pending, cx);
        }
        cx.notify();
    }

    /// The user sent tab `tab` somewhere else (typed an address, opened a
    /// bookmark): a submission still waiting for its result went nowhere,
    /// and the save offer is about the page being left.
    pub fn passwords_user_navigated(&mut self, tab: TabId, cx: &mut Context<Self>) {
        let Some(view) = tab_view(self, tab).map(|v| v.id()) else { return };
        if !cx.has_global::<Passwords>() {
            return;
        }
        if let Some(page) = Passwords::get(cx).pages.get_mut(&view) {
            page.pending = None;
            page.offer = None;
            page.popover = None;
        }
    }

    /// The page in front: the popup while one is open, else the active tab.
    fn passwords_front_view(&self) -> Option<WebView> {
        match &self.popup {
            Some(popup) => Some(popup.view.clone()),
            None => self.active_live().map(|l| l.view.clone()),
        }
    }

    fn passwords_on_page_event(&mut self, view: WebView, ev: &mut ViewEvent, cx: &mut Context<Self>) {
        let id = view.id();
        match ev {
            ViewEvent::LoadEnd { .. } => {
                let p = Passwords::get(cx);
                p.pages.retain(|v, _| WebView::by_id(*v).is_some());
                p.agents.retain(|v| WebView::by_id(*v).is_some());
                if p.agents.insert(id) {
                    install_agent(&view);
                }
                if p.status == VaultStatus::Unknown && !p.probing {
                    p.probing = true;
                    Passwords::probe(cx);
                }
                if Passwords::get(cx).page(id).is_some_and(|pg| pg.pending.as_ref().is_some_and(|s| s.navigated)) {
                    cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(RESULT_DELAY).await;
                        let _ = this.update(cx, |this, cx| this.passwords_judge_submission(id, cx));
                    })
                    .detach();
                }
            }
            ViewEvent::LoadingStateChanged { loading: true, .. } => {
                // A new document: its agent reports its own forms.
                let page = Passwords::get(cx).page_mut(id);
                page.login_form = false;
                page.new_password_form = false;
                page.focus = None;
                page.dropdown = false;
                page.confirm = None;
                page.suggestion = None;
                if page.popover == Some(PillPopover::Logins) {
                    page.popover = None;
                }
                // The offer belongs to the page the sign-in led to.
                if page.offer.as_ref().is_some_and(|o| o.at.elapsed() > OFFER_GRACE) {
                    page.offer = None;
                    page.popover = None;
                }
            }
            ViewEvent::AddressChanged(_) => {
                let page = Passwords::get(cx).page_mut(id);
                page.dropdown = false;
                page.focus = None;
                if let Some(pending) = &mut page.pending {
                    if pending.at.elapsed() < SUBMIT_WINDOW {
                        pending.navigated = true;
                    } else {
                        page.pending = None;
                    }
                }
            }
            ViewEvent::BindingCalled { name, payload } if name == AGENT_BINDING => {
                let mut payload = std::mem::take(&mut payload.0);
                let report = parse_report(&payload);
                zeroize::Zeroize::zeroize(&mut payload);
                if let Some(report) = report {
                    self.passwords_on_report(id, report, cx);
                }
            }
            _ => {}
        }
    }

    fn passwords_on_report(&mut self, id: ViewId, report: FormReport, cx: &mut Context<Self>) {
        match report {
            FormReport::Forms { origin, login, new_password } => {
                let page = Passwords::get(cx).page_mut(id);
                page.origin = Some(origin);
                page.login_form = login;
                page.new_password_form = new_password;
            }
            FormReport::Focus { origin, field, rect, max_length, user } => {
                let in_front = self.passwords_front_view().is_some_and(|v| v.id() == id);
                let p = Passwords::get(cx);
                let has_logins = p.store.has_logins_for(&origin);
                let page = p.page_mut(id);
                page.origin = Some(origin);
                page.focus = Some(FieldFocus { kind: field, rect });
                page.confirm = None;
                if field == FieldKind::NewPassword && page.suggestion.is_none() {
                    page.suggestion = generate_password(max_length.map(|m| m as usize));
                }
                let offer = match field {
                    FieldKind::NewPassword => page.suggestion.is_some(),
                    _ => has_logins,
                };
                // Only for the user's own focus, not a page's autofocus.
                if user {
                    page.dropdown = offer;
                }
                if page.dropdown && in_front {
                    // One list of logins at a time: the dropdown replaces
                    // the key button's.
                    page.popover = page.popover.filter(|p| *p == PillPopover::Offer);
                }
            }
            FormReport::Moved { rect } => {
                if let Some(focus) = &mut Passwords::get(cx).page_mut(id).focus {
                    focus.rect = rect;
                }
            }
            FormReport::Blur => {
                let page = Passwords::get(cx).page_mut(id);
                page.focus = None;
                page.dropdown = false;
                page.confirm = None;
            }
            FormReport::Typing => {
                let page = Passwords::get(cx).page_mut(id);
                page.dropdown = false;
                page.confirm = None;
            }
            FormReport::Username { origin, username } => {
                Passwords::get(cx).page_mut(id).last_username = Some((origin, username, Instant::now()));
                return;
            }
            FormReport::Submit { origin, mut username, password } => {
                let page = Passwords::get(cx).page_mut(id);
                if username.is_empty()
                    && let Some((o, u, at)) = &page.last_username
                    && *o == origin
                    && at.elapsed() < USERNAME_STEP_WINDOW
                {
                    username = u.clone();
                }
                page.pending = Some(Submission { origin, username, password, at: Instant::now(), navigated: false });
                page.dropdown = false;
                return;
            }
            FormReport::Submitted { origin } => {
                let pending = Passwords::get(cx).page_mut(id).pending.take();
                match pending {
                    Some(s) if s.origin == origin && s.at.elapsed() < SUBMIT_WINDOW => self.passwords_offer(id, s, cx),
                    other => Passwords::get(cx).page_mut(id).pending = other,
                }
            }
        }
        cx.notify();
    }

    /// A submission led to a new page: if that page shows a login form
    /// again, the sign-in most likely failed; otherwise offer to save.
    fn passwords_judge_submission(&mut self, id: ViewId, cx: &mut Context<Self>) {
        let page = Passwords::get(cx).page_mut(id);
        let still_login = page.login_form;
        let Some(pending) = page.pending.take_if(|s| s.navigated) else { return };
        if !still_login && pending.at.elapsed() < SUBMIT_WINDOW {
            self.passwords_offer(id, pending, cx);
        }
    }

    /// Offer to save (or update) a login that just worked.
    fn passwords_offer(&mut self, id: ViewId, s: Submission, cx: &mut Context<Self>) {
        let p = Passwords::get(cx);
        let target = p.store.save_target(&s.origin, &s.username);
        if target == SaveTarget::Never {
            return;
        }
        if let VaultStatus::Unavailable(reason) = &p.status {
            if !p.warned_unavailable {
                p.warned_unavailable = true;
                let text = format!("Mori can't save this password: {reason}");
                self.toast(text, ToastStyle::Warning, None, cx);
            }
            return;
        }
        match target {
            SaveTarget::New => self.passwords_show_offer(id, s, OfferKind::Save, cx),
            SaveTarget::Existing(cred) => {
                Passwords::vault_call(
                    cx,
                    move |ss| vault::fetch(ss, cred),
                    move |this, result, cx| match result {
                        // Signed in with the saved password: nothing to ask.
                        Ok(Some(saved)) if saved == s.password => {
                            let p = Passwords::get(cx);
                            p.store.mark_used(cred, mori_core::paths::now());
                            p.save();
                        }
                        Ok(_) => this.passwords_show_offer(id, s, OfferKind::Update(cred), cx),
                        Err(e) => this.toast(e, ToastStyle::Error, None, cx),
                    },
                );
            }
            SaveTarget::Never => {}
        }
    }

    fn passwords_show_offer(&mut self, id: ViewId, s: Submission, kind: OfferKind, cx: &mut Context<Self>) {
        let p = Passwords::get(cx);
        let serial = p.next_serial;
        p.next_serial += 1;
        let page = p.page_mut(id);
        page.offer = Some(SaveOffer {
            origin: s.origin,
            username: s.username,
            password: s.password,
            kind,
            serial,
            at: Instant::now(),
        });
        page.popover = Some(PillPopover::Offer);
        page.dropdown = false;
        cx.notify();
    }

    /// The front page's view (see `passwords_front_view`) and its login
    /// state.
    pub(crate) fn passwords_active<'a>(&self, cx: &'a App) -> Option<(ViewId, Option<&'a PageLogins>)> {
        let view = self.passwords_front_view()?.id();
        Some((view, cx.try_global::<Passwords>().and_then(|p| p.page(view))))
    }

    /// Save (or update) the offered login with `username`.
    pub fn passwords_accept_offer(&mut self, view: ViewId, username: String, cx: &mut Context<Self>) {
        let page = Passwords::get(cx).page_mut(view);
        page.popover = None;
        let Some(offer) = page.offer.take() else { return };
        let username = username.trim().to_string();
        let p = Passwords::get(cx);
        // The username was edited into one that is already saved: update that.
        let kind = match (offer.kind, p.store.find(&offer.origin, &username)) {
            (_, Some(existing)) => OfferKind::Update(existing.id),
            (kind, None) => kind,
        };
        let id = match kind {
            OfferKind::Update(id) => id,
            OfferKind::Save => Uuid::new_v4(),
        };
        let (origin, password) = (offer.origin.clone(), offer.password.clone());
        let user = username.clone();
        Passwords::vault_call(
            cx,
            move |ss| vault::store(ss, id, &origin, &user, &password),
            move |this, result, cx| {
                if let Err(e) = result {
                    this.toast(format!("Couldn't save the password. {e}"), ToastStyle::Error, None, cx);
                    return;
                }
                let now = mori_core::paths::now();
                let p = Passwords::get(cx);
                match kind {
                    OfferKind::Update(id) => {
                        p.store.set_username(id, &username);
                        p.store.mark_updated(id, now);
                        p.store.mark_used(id, now);
                    }
                    OfferKind::Save => {
                        p.store.add(id, &offer.origin, &username, now);
                    }
                }
                p.status = VaultStatus::Ready;
                p.save();
                let text = if matches!(kind, OfferKind::Update(_)) { "Password updated" } else { "Password saved" };
                this.toast(text, ToastStyle::Success, None, cx);
                cx.notify();
            },
        );
        cx.notify();
    }

    /// Close the save popover; the key button can bring it back until the
    /// page changes.
    pub fn passwords_dismiss_popover(&mut self, view: ViewId, cx: &mut Context<Self>) {
        Passwords::get(cx).page_mut(view).popover = None;
        cx.notify();
    }

    /// Drop the offer and never offer saving on its site again.
    pub fn passwords_never_for_site(&mut self, view: ViewId, cx: &mut Context<Self>) {
        let p = Passwords::get(cx);
        let page = p.page_mut(view);
        page.popover = None;
        if let Some(offer) = page.offer.take() {
            p.store.set_never_save(&offer.origin, true);
            p.save();
            let host = mori_core::passwords::display_origin(&offer.origin);
            self.toast(format!("Mori won't offer to save passwords on {host}"), ToastStyle::Info, None, cx);
        }
        cx.notify();
    }

    /// Esc: close the dropdown or the pill popover of the active page.
    pub fn passwords_escape(&mut self, cx: &mut Context<Self>) -> bool {
        let Some((view, Some(page))) = self.passwords_active(cx) else { return false };
        if page.popover.is_none() && !page.dropdown {
            return false;
        }
        let page = Passwords::get(cx).page_mut(view);
        page.popover = None;
        page.dropdown = false;
        page.confirm = None;
        cx.notify();
        true
    }

    /// Fill saved login `cred` into `view`. A login of a related site is
    /// only filled once the user confirmed (`confirmed`).
    pub fn passwords_fill(&mut self, view: ViewId, cred: Uuid, confirmed: bool, cx: &mut Context<Self>) {
        let p = Passwords::get(cx);
        let Some(page_origin) = p.page(view).and_then(|pg| pg.origin.clone()) else { return };
        let Some(c) = p.store.get(cred).cloned() else { return };
        match match_origin(&c.origin, &page_origin) {
            None => return,
            Some(OriginMatch::Related) if !confirmed => {
                p.page_mut(view).confirm = Some(cred);
                cx.notify();
                return;
            }
            Some(_) => {}
        }
        let page = p.page_mut(view);
        page.dropdown = false;
        page.popover = None;
        page.confirm = None;
        cx.notify();
        Passwords::vault_call(
            cx,
            move |ss| vault::fetch(ss, cred),
            move |this, result, cx| {
                let secret = match result {
                    Ok(Some(secret)) => secret,
                    Ok(None) => {
                        let text = "This password is no longer in your keyring. Remove it in Settings > Passwords.";
                        this.toast(text, ToastStyle::Error, None, cx);
                        return;
                    }
                    Err(e) => {
                        this.toast(e, ToastStyle::Error, None, cx);
                        return;
                    }
                };
                let Some(view_handle) = WebView::by_id(view) else { return };
                // A login saved without a username leaves the field alone.
                let username = (!c.username.is_empty()).then_some(c.username.as_str());
                let script = fill_script(FillMode::Login, &page_origin, username, &secret);
                let weak = cx.entity().downgrade();
                let async_cx = cx.to_async();
                evaluate_in_agent_world(&view_handle, script, move |result| {
                    let outcome = result.ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
                    let _ = weak.update(&mut async_cx.clone(), |this, cx| {
                        if outcome == "filled" {
                            let p = Passwords::get(cx);
                            p.store.mark_used(cred, mori_core::paths::now());
                            p.save();
                        } else if outcome == "no-form" {
                            this.toast("Couldn't find the sign-in form on this page", ToastStyle::Warning, None, cx);
                        }
                    });
                });
            },
        );
    }

    /// Fill the suggested strong password into the focused new-password
    /// form of `view`.
    pub fn passwords_use_suggestion(&mut self, view: ViewId, cx: &mut Context<Self>) {
        let page = Passwords::get(cx).page_mut(view);
        page.dropdown = false;
        let (Some(origin), Some(password)) = (page.origin.clone(), page.suggestion.clone()) else { return };
        cx.notify();
        let Some(view_handle) = WebView::by_id(view) else { return };
        let script = fill_script(FillMode::NewPassword, &origin, None, &password);
        evaluate_in_agent_world(&view_handle, script, |result| {
            if let Err(e) = result {
                log::warn!("passwords: filling the suggestion failed: {e}");
            }
        });
    }

    /// Open the key button's popover for the active page.
    pub fn passwords_toggle_popover(&mut self, cx: &mut Context<Self>) {
        let Some((view, page)) = self.passwords_active(cx) else { return };
        let next = match page {
            Some(pg) if pg.popover.is_some() => None,
            Some(pg) if pg.offer.is_some() => Some(PillPopover::Offer),
            _ => Some(PillPopover::Logins),
        };
        let page = Passwords::get(cx).page_mut(view);
        page.popover = next;
        page.dropdown = false;
        page.confirm = None;
        cx.notify();
    }
}
