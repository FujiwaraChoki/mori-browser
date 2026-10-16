//! Page-level overlays: JavaScript dialogs, permission prompts, and Peek.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{component::input::Input, *};
use mori_engine::{DialogKind, PermissionKind};

use crate::{
    browser::{Browser, PageOwner},
    theme::{ActiveTokens, metrics},
    ui::components::{favicon, h_flex, icon, icon_button, popover_surface, text_button, v_flex},
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

/// The host of `url`, or the URL itself when it has none.
fn display_host(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_else(|| url.to_string())
}

/// The dimmed layer a modal page prompt sits on, covering only the page.
fn page_scrim(content: impl IntoElement) -> Div {
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
        .child(content)
}

impl Browser {
    /// Everything `owner`'s page is asking the user, to lay over the page:
    /// a certificate interstitial, a dialog or credentials prompt, and a
    /// permission request.
    pub fn render_page_prompts(&self, owner: PageOwner, cx: &mut Context<Self>) -> Vec<AnyElement> {
        [
            self.render_certificate_error(owner, cx),
            self.render_js_dialog(owner, cx),
            self.render_auth(owner, cx),
            self.render_permission(owner, cx),
        ]
        .into_iter()
        .flatten()
        .collect()
    }

    fn render_js_dialog(&self, owner: PageOwner, cx: &mut Context<Self>) -> Option<AnyElement> {
        let pending = self.dialogs.front(owner)?;
        let dialog = &pending.dialog;
        let id = dialog.id;
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
        let input = pending.input.clone();
        let (ok_label, cancel_label) = match dialog.kind {
            DialogKind::BeforeUnload => ("Leave", "Stay"),
            _ => ("OK", "Cancel"),
        };
        let mut surface = popover_surface(cx)
            .w(px(420.))
            .p_4()
            .shadow(t.modal_shadow());
        if input.is_none() {
            // Alert/confirm take the keyboard: Enter is OK, Escape cancels
            // (through the window's Dismiss action).
            surface = surface.track_focus(&self.dialog_focus).on_key_down(cx.listener(
                move |this, ev: &KeyDownEvent, window, cx| {
                    if ev.keystroke.key == "enter" {
                        cx.stop_propagation();
                        this.resolve_dialog(id, true, window, cx);
                    }
                },
            ));
        }
        Some(
            page_scrim(
                surface.child(
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
                                        text_button("dialog-cancel", cancel_label, false, cx).on_click(
                                            cx.listener(move |this, _, window, cx| {
                                                this.resolve_dialog(id, false, window, cx)
                                            }),
                                        ),
                                    )
                                })
                                .child(text_button("dialog-ok", ok_label, true, cx).on_click(
                                    cx.listener(move |this, _, window, cx| {
                                        this.resolve_dialog(id, true, window, cx)
                                    }),
                                )),
                        ),
                ),
            )
            .into_any_element(),
        )
    }

    fn render_auth(&self, owner: PageOwner, cx: &mut Context<Self>) -> Option<AnyElement> {
        // A dialog from the same page goes first.
        if self.dialogs.has(owner) {
            return None;
        }
        let pending = self.auth_prompts.front(owner)?;
        let req = &pending.request;
        let id = req.id;
        let t = cx.tokens().clone();
        let target = if matches!(req.port, 80 | 443) {
            req.host.clone()
        } else {
            format!("{}:{}", req.host, req.port)
        };
        let subtitle = if req.is_proxy {
            format!("The proxy {target} requires a username and password.")
        } else {
            format!("{target} requires a username and password.")
        };
        let insecure = !req.is_proxy && req.origin.starts_with("http:");
        Some(
            page_scrim(
                popover_surface(cx).w(px(380.)).p_4().shadow(t.modal_shadow()).child(
                    v_flex()
                        .gap_3()
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_size(metrics::TEXT_TITLE)
                                .child("Sign in"),
                        )
                        .child(
                            v_flex()
                                .gap_1()
                                .text_color(t.foreground.opacity(0.85))
                                .child(subtitle)
                                .when(!req.realm.is_empty(), |s| {
                                    s.child(
                                        div()
                                            .text_size(metrics::TEXT_LABEL)
                                            .text_color(t.muted_foreground)
                                            .child(format!("“{}”", req.realm)),
                                    )
                                })
                                .when(insecure, |s| {
                                    s.child(
                                        h_flex()
                                            .gap_1()
                                            .text_size(metrics::TEXT_LABEL)
                                            .text_color(t.status_warning)
                                            .child(icon("triangle-alert", px(12.), t.status_warning))
                                            .child("Your connection to this site is not private"),
                                    )
                                }),
                        )
                        .child(Input::new(&pending.user))
                        .child(Input::new(&pending.password))
                        .child(
                            h_flex()
                                .justify_end()
                                .gap_2()
                                .child(text_button("auth-cancel", "Cancel", false, cx).on_click(
                                    cx.listener(move |this, _, window, cx| {
                                        this.resolve_auth(id, false, window, cx)
                                    }),
                                ))
                                .child(text_button("auth-ok", "Sign in", true, cx).on_click(
                                    cx.listener(move |this, _, window, cx| {
                                        this.resolve_auth(id, true, window, cx)
                                    }),
                                )),
                        ),
                ),
            )
            .into_any_element(),
        )
    }

    fn render_permission(&self, owner: PageOwner, cx: &mut Context<Self>) -> Option<AnyElement> {
        let req = self.permissions.front(owner)?;
        let id = req.id;
        let t = cx.tokens().clone();
        let origin = display_host(&req.origin);
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
                                    cx.listener(move |this, _, _, cx| this.resolve_permission(id, false, cx)),
                                ))
                                .child(text_button("perm-allow", "Allow", true, cx).on_click(
                                    cx.listener(move |this, _, _, cx| this.resolve_permission(id, true, cx)),
                                )),
                        ),
                )
                .into_any_element(),
        )
    }

    /// Chrome-style "Your connection isn't private" page, covering the page
    /// until the user goes back or chooses to continue.
    fn render_certificate_error(&self, owner: PageOwner, cx: &mut Context<Self>) -> Option<AnyElement> {
        let err = self.cert_errors.front(owner)?;
        let id = err.id;
        let t = cx.tokens().clone();
        let host = display_host(&err.url);
        let (name, reason) = mori_core::prompts::certificate_error_text(err.code);
        Some(
            div()
                .id("cert-error")
                .absolute()
                .inset_0()
                .occlude()
                .flex()
                .items_center()
                .justify_center()
                .bg(t.card)
                .rounded(metrics::RADIUS)
                .child(
                    v_flex()
                        .max_w(px(440.))
                        .gap_3()
                        .child(icon("shield-alert", px(36.), t.destructive))
                        .child(
                            div()
                                .text_size(px(20.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("Your connection isn't private"),
                        )
                        .child(div().text_color(t.foreground.opacity(0.85)).child(format!(
                            "Attackers might be trying to steal your information from {host} \
                             (for example, passwords, messages, or credit cards)."
                        )))
                        .child(
                            div()
                                .text_size(metrics::TEXT_LABEL)
                                .text_color(t.muted_foreground)
                                .child(reason),
                        )
                        .child(
                            div()
                                .font_family("monospace")
                                .text_size(metrics::TEXT_SMALL)
                                .text_color(t.muted_foreground)
                                .child(name),
                        )
                        .child(
                            h_flex()
                                .pt_2()
                                .gap_2()
                                .justify_between()
                                .child(
                                    div()
                                        .id("cert-proceed")
                                        .text_size(metrics::TEXT_LABEL)
                                        .text_color(t.muted_foreground)
                                        .underline()
                                        .cursor_pointer()
                                        .hover(|s| s.text_color(t.foreground))
                                        .child(format!("Continue to {host} (unsafe)"))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.resolve_certificate_error(id, true, cx)
                                        })),
                                )
                                .child(text_button("cert-back", "Back to safety", true, cx).on_click(
                                    cx.listener(move |this, _, _, cx| {
                                        this.resolve_certificate_error(id, false, cx)
                                    }),
                                )),
                        ),
                )
                .into_any_element(),
        )
    }

    pub fn render_peek(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let peek = self.peek.as_ref()?;
        let t = cx.tokens().clone();
        let host = mori_core::util::host_of(&peek.url);
        let fav = host.as_deref().and_then(|h| self.favicons.get(h));
        let title = if peek.title.is_empty() { peek.url.clone() } else { peek.title.clone() };
        let url = mori_core::omnibox::display_url(&peek.url);
        let surface = peek.surface.clone();
        let copy_url = peek.url.clone();
        let prompts = self.render_page_prompts(PageOwner::Peek, cx);
        Some(
            div()
                .absolute()
                .inset_0()
                .occlude()
                .bg(black().opacity(0.32))
                .flex()
                .items_center()
                .justify_center()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| this.close_peek(window, cx)),
                )
                .child(
                    v_flex()
                        .w(relative(0.74))
                        .max_w(px(1000.))
                        .h(relative(0.82))
                        .max_h(px(820.))
                        .rounded(metrics::RADIUS)
                        .bg(t.card)
                        .border_1()
                        .border_color(t.border.opacity(0.7))
                        .shadow(t.overlay_shadow())
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            h_flex()
                                .h(px(46.))
                                .flex_none()
                                .px_3()
                                .gap_2()
                                .child(favicon(fav, px(16.), false, cx))
                                .child(
                                    v_flex()
                                        .flex_1()
                                        .min_w_0()
                                        .child(
                                            div()
                                                .truncate()
                                                .font_weight(FontWeight::MEDIUM)
                                                .child(title),
                                        )
                                        .child(
                                            div()
                                                .truncate()
                                                .text_size(metrics::TEXT_SMALL)
                                                .text_color(t.muted_foreground)
                                                .child(url),
                                        ),
                                )
                                .child(icon_button("peek-copy", "link", px(28.), true, cx).on_click(
                                    cx.listener(move |this, _, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(copy_url.clone()));
                                        this.toast(
                                            "Copied URL",
                                            crate::browser::ToastStyle::Success,
                                            None,
                                            cx,
                                        );
                                    }),
                                ))
                                .child(
                                    text_button("peek-open", "Open in Space", true, cx).on_click(
                                        cx.listener(|this, _, window, cx| this.promote_peek(window, cx)),
                                    ),
                                )
                                .child(icon_button("peek-close", "x", px(28.), true, cx).on_click(
                                    cx.listener(|this, _, window, cx| this.close_peek(window, cx)),
                                )),
                        )
                        .child(
                            div()
                                .relative()
                                .flex_1()
                                .min_h_0()
                                .p(px(1.))
                                .child(surface)
                                .children(prompts),
                        ),
                )
                .into_any_element(),
        )
    }
}
