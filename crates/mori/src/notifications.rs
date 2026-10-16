//! Web notifications on the desktop, through the freedesktop notification
//! service (`org.freedesktop.Notifications` on the session bus).
//!
//! Pages create notifications through the page bridge (see
//! `mori_core::privacy`); [`Notifier::show`] sends them to the desktop and
//! remembers which tab each came from. Clicking one brings Mori forward on
//! that tab and fires the page's `click` handler; closing it fires `close`.

use std::{collections::HashMap, path::PathBuf, sync::OnceLock};

use futures::StreamExt as _;
use gpui_kit::*;
use mori_core::workspace::TabId;

use crate::browser::Browser;

const DESTINATION: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
const INTERFACE: &str = "org.freedesktop.Notifications";

/// A notification a page asked for.
pub struct WebNotification {
    pub browser: WeakEntity<Browser>,
    pub tab: TabId,
    /// The page's id for it (answers go back with it).
    pub nid: u64,
    /// The site, shown as the sending app.
    pub source: String,
    pub title: String,
    pub body: String,
    /// Same origin and tag replace the earlier notification.
    pub tag: Option<String>,
    pub icon: Option<PathBuf>,
    pub silent: bool,
}

/// What the desktop reported about a notification.
enum Signal {
    Clicked(u32),
    Closed(u32),
}

struct Shown {
    browser: WeakEntity<Browser>,
    tab: TabId,
    nid: u64,
    tag_key: Option<String>,
}

/// Notifications on screen, by the server's id.
#[derive(Default)]
pub struct Notifier {
    shown: HashMap<u32, Shown>,
    by_tag: HashMap<String, u32>,
    listening: bool,
}

impl Global for Notifier {}

/// The session bus connection, opened on first use (`None` without a bus).
fn connection() -> Option<&'static zbus::blocking::Connection> {
    static CONNECTION: OnceLock<Option<zbus::blocking::Connection>> = OnceLock::new();
    CONNECTION
        .get_or_init(|| match zbus::blocking::Connection::session() {
            Ok(c) => Some(c),
            Err(e) => {
                log::warn!("no session bus for notifications: {e}");
                None
            }
        })
        .as_ref()
}

impl Notifier {
    /// Put a page's notification on the desktop.
    pub fn show(note: WebNotification, cx: &mut App) {
        Self::listen(cx);
        let tag_key = note.tag.as_ref().filter(|t| !t.is_empty()).map(|t| format!("{}\u{0}{t}", note.source));
        let replaces = tag_key
            .as_ref()
            .and_then(|k| cx.default_global::<Notifier>().by_tag.get(k).copied())
            .unwrap_or(0);
        let WebNotification { browser, tab, nid, source, title, body, icon, silent, .. } = note;
        let task = cx.background_executor().spawn(async move {
            let conn = connection()?;
            let icon = icon.map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
            let mut hints: HashMap<&str, zbus::zvariant::Value<'_>> = HashMap::new();
            hints.insert("desktop-entry", zbus::zvariant::Value::from("mori"));
            if silent {
                hints.insert("suppress-sound", zbus::zvariant::Value::from(true));
            }
            let actions = vec!["default", "Open"];
            let reply = conn.call_method(
                Some(DESTINATION),
                PATH,
                Some(INTERFACE),
                "Notify",
                &(source.as_str(), replaces, icon.as_str(), title.as_str(), body.as_str(), actions, hints, -1i32),
            );
            match reply.and_then(|m| m.body().deserialize::<u32>()) {
                Ok(id) => Some(id),
                Err(e) => {
                    log::warn!("showing a notification failed: {e}");
                    None
                }
            }
        });
        cx.spawn(async move |cx| {
            let Some(id) = task.await else { return };
            cx.update(|cx| {
                let notifier = cx.default_global::<Notifier>();
                if let Some(key) = &tag_key {
                    notifier.by_tag.insert(key.clone(), id);
                }
                notifier.shown.insert(id, Shown { browser, tab, nid, tag_key });
            });
        })
        .detach();
    }

    /// Take a page's notification off the desktop (`notification.close()`).
    pub fn close(tab: TabId, nid: u64, cx: &mut App) {
        let notifier = cx.default_global::<Notifier>();
        let Some(id) = notifier.shown.iter().find(|(_, s)| s.tab == tab && s.nid == nid).map(|(id, _)| *id) else {
            return;
        };
        notifier.forget(id);
        cx.background_executor()
            .spawn(async move {
                if let Some(conn) = connection() {
                    let _ = conn.call_method(Some(DESTINATION), PATH, Some(INTERFACE), "CloseNotification", &(id,));
                }
            })
            .detach();
    }

    /// Notifications of a closed tab can no longer be answered.
    pub fn forget_tab(tab: TabId, cx: &mut App) {
        let notifier = cx.default_global::<Notifier>();
        let ids: Vec<u32> = notifier.shown.iter().filter(|(_, s)| s.tab == tab).map(|(id, _)| *id).collect();
        for id in ids {
            notifier.forget(id);
        }
    }

    fn forget(&mut self, id: u32) -> Option<Shown> {
        let shown = self.shown.remove(&id)?;
        if let Some(key) = &shown.tag_key
            && self.by_tag.get(key) == Some(&id)
        {
            self.by_tag.remove(key);
        }
        Some(shown)
    }

    /// Start listening for clicks and closes (once).
    fn listen(cx: &mut App) {
        let notifier = cx.default_global::<Notifier>();
        if std::mem::replace(&mut notifier.listening, true) {
            return;
        }
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<Signal>();
        std::thread::Builder::new()
            .name("mori-notifications".into())
            .spawn(move || {
                let Some(conn) = connection() else { return };
                // One stream for both signals: servers send `ActionInvoked`
                // and then `NotificationClosed` for a click, and the click
                // must be seen first (the close forgets the notification).
                let rule = format!("type='signal',interface='{INTERFACE}',path='{PATH}'");
                let signals = match zbus::blocking::MessageIterator::for_match_rule(rule.as_str(), conn, None) {
                    Ok(s) => s,
                    Err(e) => {
                        log::warn!("notification signals unavailable: {e}");
                        return;
                    }
                };
                for msg in signals.flatten() {
                    let header = msg.header();
                    let signal = match header.member().map(|m| m.as_str()) {
                        Some("ActionInvoked") => match msg.body().deserialize::<(u32, String)>() {
                            Ok((id, action)) if action == "default" => Signal::Clicked(id),
                            _ => continue,
                        },
                        Some("NotificationClosed") => match msg.body().deserialize::<(u32, u32)>() {
                            Ok((id, _reason)) => Signal::Closed(id),
                            Err(_) => continue,
                        },
                        _ => continue,
                    };
                    if tx.unbounded_send(signal).is_err() {
                        break;
                    }
                }
            })
            .ok();
        cx.spawn(async move |cx| {
            while let Some(signal) = rx.next().await {
                cx.update(|cx| Self::on_signal(signal, cx));
            }
        })
        .detach();
    }

    fn on_signal(signal: Signal, cx: &mut App) {
        let (id, clicked) = match signal {
            Signal::Clicked(id) => (id, true),
            Signal::Closed(id) => (id, false),
        };
        let Some(shown) = cx.default_global::<Notifier>().forget(id) else { return };
        let Some(browser) = shown.browser.upgrade() else { return };
        let window = browser.read(cx).window_handle();
        let (tab, nid) = (shown.tab, shown.nid);
        window
            .update(cx, |_, window, cx| {
                browser.update(cx, |b, cx| b.on_notification_answer(tab, nid, clicked, window, cx));
                if clicked {
                    window.activate_window();
                }
            })
            .ok();
    }
}
