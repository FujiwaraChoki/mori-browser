//! Page-level overlays: JavaScript dialogs and permission prompts.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{component::input::Input, *};
use mori_engine::{DialogKind, PermissionKind};

use crate::{
    browser::Browser,
    theme::{ActiveTokens, metrics},
    ui::components::{h_flex, popover_surface, text_button, v_flex},
};

fn permission_label(kind: PermissionKind) -> &'static str {
    match kind {
        PermissionKind::Camera => "Use your camera",
        PermissionKind::Microphone => "Use your microphone",
        PermissionKind::Geolocation => "Know your location",
        PermissionKind::Notifications => "Show notifications",
        PermissionKind::Clipboard => "See text and images copied to the clipboard",
        PermissionKind::Midi => "Use your MIDI devices",
        PermissionKind::Sensors => "Use motion sensors",
        PermissionKind::ScreenCapture => "Share your screen",
        PermissionKind::Storage => "Use cookies and site data across sites",
        PermissionKind::WindowManagement => "Manage windows on your displays",
        PermissionKind::LocalFonts => "Use your computer's fonts",
        PermissionKind::IdleDetection => "Know when you're actively using this device",
        PermissionKind::Other => "Access a device feature",
    }
}

impl Browser {
    pub fn render_js_dialog(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let selected = self.selected()?;
        let (_, dialog) = self.dialogs.iter().find(|(t, _)| *t == selected)?;
        let t = cx.tokens().clone();
        let is_alert = dialog.kind == DialogKind::Alert;
        let title = match dialog.kind {
            DialogKind::BeforeUnload => "Leave site?".to_string(),
            _ => url::Url::parse(&dialog.origin)
                .ok()
                .and_then(|u| u.host_str().map(|h| format!("{h} says")))
                .unwrap_or_else(|| "This page says".into()),
        };
        let message = if dialog.kind == DialogKind::BeforeUnload {
            "Changes you made may not be saved.".to_string()
        } else {
            dialog.message.clone()
        };
        let input = self.dialog_input.clone();
        let (ok_label, cancel_label) = match dialog.kind {
            DialogKind::BeforeUnload => ("Leave", "Stay"),
            _ => ("OK", "Cancel"),
        };
        Some(
            div()
                .absolute()
                .inset_0()
                .occlude()
                .rounded(metrics::RADIUS)
                .bg(black().opacity(0.28))
                .flex()
                .items_start()
                .justify_center()
                .pt(px(80.))
                .child(
                    popover_surface(cx)
                        .w(px(420.))
                        .p_4()
                        .shadow(t.modal_shadow())
                        .child(
                            v_flex()
                                .gap_3()
                                .child(
                                    div()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_size(metrics::TEXT_TITLE)
                                        .child(title),
                                )
                                .child(
                                    div()
                                        .id("dialog-message")
                                        .max_h(px(240.))
                                        .overflow_y_scroll()
                                        .text_color(t.foreground.opacity(0.85))
                                        .child(message),
                                )
                                .when_some(input, |s, input| s.child(Input::new(&input)))
                                .child(
                                    h_flex()
                                        .justify_end()
                                        .gap_2()
                                        .when(!is_alert, |s| {
                                            s.child(
                                                text_button("dialog-cancel", cancel_label, false, cx)
                                                    .on_click(cx.listener(|this, _, window, cx| {
                                                        this.resolve_dialog(false, window, cx)
                                                    })),
                                            )
                                        })
                                        .child(text_button("dialog-ok", ok_label, true, cx).on_click(
                                            cx.listener(|this, _, window, cx| {
                                                this.resolve_dialog(true, window, cx)
                                            }),
                                        )),
                                ),
                        ),
                )
                .into_any_element(),
        )
    }

    pub fn render_permission(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let selected = self.selected()?;
        let (_, req) = self.permissions.iter().find(|(t, _)| *t == selected)?;
        let t = cx.tokens().clone();
        let origin = url::Url::parse(&req.origin)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
            .unwrap_or_else(|| req.origin.clone());
        Some(
            popover_surface(cx)
                .id("permission-prompt")
                .absolute()
                .top(px(14.))
                .right(px(14.))
                .w(px(332.))
                .p_4()
                .occlude()
                .child(
                    v_flex()
                        .gap_3()
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(format!("{origin} wants to")),
                        )
                        .child(v_flex().gap_1().children(req.kinds.iter().map(|k| {
                            h_flex()
                                .gap_2()
                                .text_color(t.foreground.opacity(0.85))
                                .child(div().size(px(4.)).rounded_full().bg(t.muted_foreground))
                                .child(permission_label(*k))
                        })))
                        .child(
                            h_flex()
                                .justify_end()
                                .gap_2()
                                .child(text_button("perm-block", "Block", false, cx).on_click(
                                    cx.listener(|this, _, _, cx| this.resolve_permission(false, cx)),
                                ))
                                .child(text_button("perm-allow", "Allow", true, cx).on_click(
                                    cx.listener(|this, _, _, cx| this.resolve_permission(true, cx)),
                                )),
                        ),
                )
                .into_any_element(),
        )
    }
}
