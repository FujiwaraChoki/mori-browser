//! A message channel from page script to the chrome.
//!
//! The chrome registers a script that runs in every document before the
//! page's own (DevTools `Page.addScriptToEvaluateOnNewDocument`) and talks
//! back through console messages that start with a marker. (A DevTools
//! binding would need the Runtime domain enabled for every page, which
//! streams every console call and exception through the browser.) Every
//! message must carry a per-view secret the script got baked in, so other
//! script on the page, which can log the marker too, cannot speak for it.
//!
//! Permission requests (`{t: "perm", kind}`) become ordinary
//! [`ViewEvent::PermissionPrompt`]s: Chromium itself denies every prompt
//! under headless ozone, so this is how pages reach Mori's prompt. Other
//! messages arrive as [`ViewEvent::PageMessage`].

use std::{
    cell::RefCell,
    hash::{BuildHasher, Hasher},
    rc::{Rc, Weak},
};

use crate::view::{PendingPermission, PermissionId, PermissionKind, PermissionRequest, ViewEvent, ViewInner, next_token};

/// How the chrome's page script and the engine talk.
#[derive(Clone, Debug)]
pub struct PageBridge {
    /// Console messages starting with this are bridge messages (JSON
    /// follows).
    pub console_marker: String,
    /// The script; `token_placeholder` in it is replaced by the view's
    /// secret.
    pub script: String,
    pub token_placeholder: String,
    /// Page function answering a request: `reply(rid, value)`.
    pub reply_function: String,
    /// Page function delivering an event object: `event(value)`.
    pub event_function: String,
}

thread_local! {
    static BRIDGE: RefCell<Option<Rc<PageBridge>>> = const { RefCell::new(None) };
}

/// Install `bridge` for every view, now and later. Documents already
/// loaded keep their old script until they navigate.
pub fn set_page_bridge(bridge: PageBridge) {
    BRIDGE.with(|b| *b.borrow_mut() = Some(Rc::new(bridge)));
    for view in crate::view::all_views() {
        if view.browser.borrow().is_some() {
            install(&view);
        }
    }
}

fn bridge() -> Option<Rc<PageBridge>> {
    BRIDGE.with(|b| b.borrow().clone())
}

/// A fresh unguessable secret for a view.
fn new_token() -> String {
    let state = std::collections::hash_map::RandomState::new();
    let mut parts = String::new();
    for i in 0..2u64 {
        let mut h = state.build_hasher();
        h.write_u64(i ^ next_token());
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
        parts.push_str(&format!("{:016x}", h.finish()));
    }
    parts
}

/// Register the bridge with a view whose browser now exists.
pub(crate) fn attach(view: &Rc<ViewInner>) {
    install(view);
}

fn install(view: &Rc<ViewInner>) {
    let Some(bridge) = bridge() else { return };
    if view.bridge_token.borrow().is_empty() {
        *view.bridge_token.borrow_mut() = new_token();
    }
    let source = bridge.script.replace(&bridge.token_placeholder, &view.bridge_token.borrow());
    let id = view.id;
    // New-document scripts only run while the Page domain is enabled.
    crate::devtools::call(view, "Page.enable", serde_json::json!({}), Box::new(|_| {}));
    if let Some(old) = view.bridge_script_id.borrow_mut().take() {
        crate::devtools::call(
            view,
            "Page.removeScriptToEvaluateOnNewDocument",
            serde_json::json!({ "identifier": old }),
            Box::new(|_| {}),
        );
    }
    // The document on screen may have started before the script was
    // registered (a new browser's first page): give it the bridge now. The
    // script does nothing where it already ran.
    run(view, &source);
    let weak: Weak<ViewInner> = Rc::downgrade(view);
    crate::devtools::call(
        view,
        "Page.addScriptToEvaluateOnNewDocument",
        serde_json::json!({ "source": source }),
        Box::new(move |r| match r {
            Ok(result) => {
                if let (Some(view), Some(identifier)) = (weak.upgrade(), result.get("identifier").and_then(|i| i.as_str())) {
                    *view.bridge_script_id.borrow_mut() = Some(identifier.to_string());
                }
            }
            Err(e) => log::warn!("view {id:?}: installing the page bridge failed: {e}"),
        }),
    );
}

/// Handle a console message if it is a bridge message; returns whether it
/// was one (it is then not logged).
pub(crate) fn on_console_message(view: &Rc<ViewInner>, message: &str) -> bool {
    let Some(bridge) = bridge() else { return false };
    let Some(payload) = message.strip_prefix(bridge.console_marker.as_str()) else { return false };
    if let Some(message) = authenticate(payload, &view.bridge_token.borrow()) {
        handle(view, message);
    }
    true
}

/// The message in `payload` without its token, if the token is the view's.
fn authenticate(payload: &str, token: &str) -> Option<serde_json::Value> {
    let mut value: serde_json::Value = serde_json::from_str(payload).ok()?;
    let object = value.as_object_mut()?;
    let given = object.remove("token")?;
    (!token.is_empty() && given.as_str() == Some(token)).then_some(value)
}

fn permission_kind(name: &str) -> Option<PermissionKind> {
    Some(match name {
        "notifications" => PermissionKind::Notifications,
        "geolocation" => PermissionKind::Geolocation,
        "clipboard" => PermissionKind::Clipboard,
        _ => return None,
    })
}

/// `scheme://host[:port]` of a URL.
fn origin(url: &str) -> String {
    match url.find("://") {
        Some(i) => {
            let rest = &url[i + 3..];
            let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
            format!("{}{}", &url[..i + 3], &rest[..end])
        }
        None => url.to_string(),
    }
}

fn handle(view: &Rc<ViewInner>, message: serde_json::Value) {
    if message.get("t").and_then(|t| t.as_str()) != Some("perm") {
        view.push(ViewEvent::PageMessage(message));
        return;
    }
    let rid = message.get("rid").and_then(|r| r.as_u64());
    let kind = message.get("kind").and_then(|k| k.as_str()).and_then(permission_kind);
    let (Some(rid), Some(kind)) = (rid, kind) else { return };
    let url = view
        .main_frame()
        .map(|f| cef::CefString::from(&cef::ImplFrame::url(&f)).to_string())
        .unwrap_or_default();
    let origin = origin(&url);
    let id = PermissionId(next_token());
    view.permissions
        .borrow_mut()
        .insert(id, PendingPermission::Bridge { rid, kind, origin: origin.clone() });
    view.push(ViewEvent::PermissionPrompt(PermissionRequest { id, origin, kinds: vec![kind] }));
}

/// Answer a bridge permission request: record the decision as a content
/// setting (so Chromium's own checks agree) and tell the page.
pub(crate) fn resolve_permission(view: &ViewInner, rid: u64, kind: PermissionKind, origin: &str, allow: bool) {
    crate::privacy::set_permission_setting(origin, kind, Some(allow));
    reply(view, rid, &serde_json::json!(if allow { "granted" } else { "denied" }));
}

/// Answer request `rid` of the page's bridge script.
pub(crate) fn reply(view: &ViewInner, rid: u64, value: &serde_json::Value) {
    let Some(bridge) = bridge() else { return };
    let f = &bridge.reply_function;
    run(view, &format!("window.{f} && window.{f}({rid}, {value})"));
}

/// Deliver an event to the page's bridge script.
pub(crate) fn event(view: &Rc<ViewInner>, value: &serde_json::Value, user_gesture: bool) {
    let Some(bridge) = bridge() else { return };
    let f = &bridge.event_function;
    let code = format!("window.{f} && window.{f}({value})");
    if user_gesture {
        // The user clicked something of the page's (a notification): its
        // handler may focus or open a window.
        crate::devtools::evaluate(view, &code, true, Box::new(|_| {}));
    } else {
        run(view, &code);
    }
}

fn run(view: &ViewInner, code: &str) {
    if let Some(frame) = view.main_frame() {
        cef::ImplFrame::execute_java_script(&frame, Some(&cef::CefString::from(code)), None, 0);
    }
}

/// Withdraw bridge prompts of a document that is going away.
pub(crate) fn dismiss_prompts(view: &ViewInner) {
    let ids: Vec<PermissionId> = view
        .permissions
        .borrow()
        .iter()
        .filter(|(_, p)| matches!(p, PendingPermission::Bridge { .. }))
        .map(|(id, _)| *id)
        .collect();
    for id in ids {
        view.permissions.borrow_mut().remove(&id);
        view.push(ViewEvent::PermissionDismissed(id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_the_token() {
        let msg = authenticate(r#"{"t":"notify","nid":1,"token":"s3cret"}"#, "s3cret").unwrap();
        assert_eq!(msg, serde_json::json!({"t": "notify", "nid": 1}));
        assert!(authenticate(r#"{"t":"notify","token":"guess"}"#, "s3cret").is_none());
        assert!(authenticate(r#"{"t":"notify"}"#, "s3cret").is_none());
        assert!(authenticate(r#"{"t":"notify","token":""}"#, "").is_none(), "no token yet");
        assert!(authenticate("not json", "s3cret").is_none());
        assert!(authenticate("[1]", "s3cret").is_none());
    }

    #[test]
    fn origins() {
        assert_eq!(origin("https://mail.google.com/mail/u/0/#inbox"), "https://mail.google.com");
        assert_eq!(origin("http://localhost:8080?x"), "http://localhost:8080");
        assert_eq!(origin("about:blank"), "about:blank");
    }

    #[test]
    fn tokens_differ() {
        let a = new_token();
        assert_eq!(a.len(), 32);
        assert_ne!(a, new_token());
    }

    #[test]
    fn permission_names() {
        assert_eq!(permission_kind("geolocation"), Some(PermissionKind::Geolocation));
        assert_eq!(permission_kind("camera"), None);
    }
}
