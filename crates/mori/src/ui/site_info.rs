//! Site info: the popover behind the URL pill's leading icon, like Zen's
//! identity panel and Arc's site controls. It shows whether the connection
//! is secure, the permissions remembered for the site (switchable and
//! resettable), page zoom, and shortcuts to Boost and copying the link.

use std::rc::Rc;

use gpui_kit::*;
use mori_core::site_permissions::Decision;
use mori_engine::PermissionKind;

use crate::{
    browser::{Browser, ToastStyle},
    commands::Command,
    theme::{ActiveTokens, metrics},
    ui::{
        components::{h_flex, icon, icon_button, v_flex},
        menus::{Entry, EntryExt, cmd, item},
    },
};

/// Anchor key of the URL pill (see [`crate::ui::menus::anchor_probe`]).
pub const OMNIBOX_ANCHOR: &str = "omnibox";

/// The name a permission is remembered under, or `None` for kinds Chromium
/// doesn't tell apart (never remembered, always asked).
pub fn permission_key(kind: PermissionKind) -> Option<&'static str> {
    Some(match kind {
        PermissionKind::Camera => "camera",
        PermissionKind::Microphone => "microphone",
        PermissionKind::Geolocation => "geolocation",
        PermissionKind::Notifications => "notifications",
        PermissionKind::Clipboard => "clipboard",
        PermissionKind::Midi => "midi",
        PermissionKind::Sensors => "sensors",
        PermissionKind::ScreenCapture => "screen-capture",
        PermissionKind::Storage => "storage",
        PermissionKind::WindowManagement => "window-management",
        PermissionKind::LocalFonts => "local-fonts",
        PermissionKind::IdleDetection => "idle-detection",
        PermissionKind::Other => return None,
    })
}

/// Label and icon for a remembered permission.
fn permission_label(key: &str) -> (&'static str, &'static str) {
    match key {
        "camera" => ("Camera", "camera"),
        "microphone" => ("Microphone", "mic"),
        "geolocation" => ("Location", "map-pin"),
        "notifications" => ("Notifications", "bell"),
        "clipboard" => ("Clipboard", "clipboard"),
        "midi" => ("MIDI devices", "piano"),
        "sensors" => ("Motion sensors", "activity"),
        "screen-capture" => ("Screen sharing", "monitor"),
        "storage" => ("Cookies across sites", "cookie"),
        "window-management" => ("Window management", "app-window"),
        "local-fonts" => ("Local fonts", "type"),
        "idle-detection" => ("Idle detection", "timer"),
        _ => ("Other", "shield"),
    }
}

/// Entries of the site-info popover for the selected page.
pub fn site_info_entries(b: &Browser) -> Vec<Entry> {
    let Some(url) = b.selected().and_then(|id| b.ws.tab(id)).map(|t| t.url.clone()) else {
        return Vec::new();
    };
    let host = mori_core::omnibox::display_host(&url);
    let (glyph, status, secure) = if url.starts_with("https://") {
        ("lock", "Connection is secure", Some(true))
    } else if url.starts_with("http://") {
        ("triangle-alert", "Connection is not secure", Some(false))
    } else if url.starts_with("file://") {
        ("file", "Local file", None)
    } else {
        ("info", "Browser page", None)
    };
    let header = {
        let host = host.clone();
        Entry::Custom {
            height: 52.,
            render: Rc::new(move |_, cx| {
                let t = cx.tokens().clone();
                let color = match secure {
                    Some(false) => t.status_warning,
                    _ => t.muted_foreground,
                };
                v_flex()
                    .h(px(52.))
                    .px(px(10.))
                    .justify_center()
                    .gap(px(2.))
                    .child(div().truncate().font_weight(FontWeight::SEMIBOLD).child(host.clone()))
                    .child(
                        h_flex()
                            .gap(px(6.))
                            .text_size(metrics::TEXT_LABEL)
                            .text_color(color)
                            .child(icon(glyph, px(12.), color))
                            .child(status),
                    )
                    .into_any_element()
            }),
        }
    };
    let mut v = vec![header];

    let is_web = url.starts_with("http://") || url.starts_with("https://");
    if is_web {
        v.push(Entry::Separator);
        v.push(Entry::Header("Permissions".into()));
        let remembered = b.site_permissions.for_site(&url);
        if remembered.is_empty() {
            v.push(Entry::Note("This site hasn't asked for any permissions.".into()));
        }
        for (key, decision) in &remembered {
            let (label, glyph) = permission_label(key);
            let (key, page) = (key.clone(), url.clone());
            v.push(Entry::Switch {
                label: label.into(),
                icon: glyph,
                on: *decision == Decision::Allow,
                handler: Rc::new(move |b, _, cx| {
                    let next = match b.site_permissions.get(&page, &key) {
                        Some(Decision::Allow) => Decision::Block,
                        _ => Decision::Allow,
                    };
                    b.site_permissions.set(&page, &key, next);
                    b.schedule_save(cx);
                    cx.notify();
                }),
            });
        }
        if !remembered.is_empty() {
            let page = url.clone();
            v.push(item("Reset Permissions", "rotate-ccw", move |b, _, cx| {
                b.site_permissions.forget_site(&page);
                b.toast("Permissions reset for this site", ToastStyle::Info, None, cx);
                b.schedule_save(cx);
            }));
        }
    }

    if b.active_live().is_some() {
        v.push(Entry::Separator);
        let level = b.selected_zoom_level();
        v.push(Entry::Custom {
            height: 36.,
            render: Rc::new(move |_, cx| {
                let t = cx.tokens().clone();
                let pct = mori_core::zoom::percent_for_level(level);
                h_flex()
                    .h(px(36.))
                    .px(px(10.))
                    .gap(px(6.))
                    .child(icon("zoom-in", px(15.), t.foreground.opacity(0.6)))
                    .child(div().flex_1().ml(px(4.)).child("Zoom"))
                    .child(
                        icon_button("site-zoom-out", "minus", px(24.), true, cx)
                            .on_click(cx.listener(|this, _, _, cx| this.zoom(-0.5, cx))),
                    )
                    .child(
                        div()
                            .id("site-zoom-reset")
                            .w(px(48.))
                            .flex()
                            .justify_center()
                            .text_size(metrics::TEXT_LABEL)
                            .text_color(t.muted_foreground)
                            .cursor_pointer()
                            .child(format!("{pct}%"))
                            .on_click(cx.listener(|this, _, _, cx| this.zoom(0.0, cx))),
                    )
                    .child(
                        icon_button("site-zoom-in", "plus", px(24.), true, cx)
                            .on_click(cx.listener(|this, _, _, cx| this.zoom(0.5, cx))),
                    )
                    .into_any_element()
            }),
        });
    }

    if is_web {
        v.push(Entry::Separator);
        v.push(cmd("Boost This Site…", "zap", Command::BoostSite));
        let link = url.clone();
        v.push(
            item("Copy Link", "link", move |b, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(link.clone()));
                b.toast("Copied link", ToastStyle::Success, None, cx);
            })
            .shortcut("secondary-shift-c"),
        );
    }
    v
}
