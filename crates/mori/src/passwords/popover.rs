//! Password UI around the page: the key button in the URL pill, the
//! popover hanging off it ("Save password?" and the site's saved logins),
//! and the suggestions dropdown under a focused login field.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    component::input::{Input, InputState},
    *,
};
use mori_core::passwords::{Credential, OriginMatch, display_origin, report::FieldKind};
use mori_engine::{ViewId, WebView};

use super::{OfferKind, Passwords, PillPopover};
use crate::{
    browser::Browser,
    theme::{ActiveTokens, metrics},
    ui::{
        components::{h_flex, icon, icon_button, popover_surface, text_button, v_flex},
        menus::{anchor_bounds, anchor_probe},
        settings::SettingsCategory,
    },
};

/// Anchor key of the key button in the URL pill.
const KEY_ANCHOR: &str = "password-key";
const POPOVER_WIDTH: f32 = 320.;
const ROW_HEIGHT: f32 = 40.;
const DROPDOWN_MIN_WIDTH: f32 = 260.;
const DROPDOWN_MAX_WIDTH: f32 = 380.;

/// Per-window state of the password UI.
#[derive(Default)]
pub struct PasswordsUi {
    /// The save popover's username field, for the offer with this serial.
    offer_input: Option<(u64, Entity<InputState>)>,
    /// The save popover shows the password in the clear.
    offer_reveal: bool,
    /// The Passwords settings page, made on first open.
    pub page: Option<super::PasswordsPage>,
}

/// One saved login offered on the page, with how its site matches.
#[derive(Clone)]
struct Offered {
    cred: Credential,
    matched: OriginMatch,
}

impl Browser {
    /// The key button for the URL pill: shown while the page has a save
    /// offer, or a login form and logins saved for it.
    pub fn password_key_button(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (_, page) = self.passwords_active(cx)?;
        let page = page?;
        let has_logins = page
            .origin
            .as_deref()
            .is_some_and(|origin| cx.try_global::<Passwords>().is_some_and(|p| p.store.has_logins_for(origin)));
        let show = page.offer.is_some() || page.popover.is_some() || ((page.login_form || page.focus.is_some()) && has_logins);
        if !show {
            return None;
        }
        let t = cx.tokens().clone();
        let open = page.popover.is_some();
        // Brighter while it has something to say (on any space wash).
        let color = if page.offer.is_some() || open { t.sidebar_foreground } else { t.sidebar_foreground.opacity(0.6) };
        Some(
            div()
                .id("password-key")
                .relative()
                .flex_none()
                .size(px(20.))
                .mx(px(-2.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(metrics::RADIUS_MD)
                .cursor_pointer()
                .hover(|s| s.bg(t.ghost_hover()))
                .when(open, |s| s.bg(t.ghost_hover()))
                .child(anchor_probe(KEY_ANCHOR))
                .child(icon("key-round", px(14.), color))
                .tooltip(|window, cx| gpui_kit::component::tooltip::Tooltip::new("Passwords").build(window, cx))
                .on_click(cx.listener(|this, _, _, cx| {
                    cx.stop_propagation();
                    this.passwords_toggle_popover(cx);
                }))
                .into_any_element(),
        )
    }

    /// The pill popover and field dropdowns to draw over the window.
    pub fn render_password_overlays(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        if self.settings_open || cx.try_global::<Passwords>().is_none() {
            return Vec::new();
        }
        let mut out = Vec::new();
        if let Some(popover) = self.render_pill_popover(window, cx) {
            out.push(popover);
        }
        // The pages on screen: the popup covers the tabs while it's open.
        let pages: Vec<(WebView, Bounds<Pixels>)> = match &self.popup {
            Some(popup) => vec![(popup.view.clone(), popup.surface.read(cx).bounds())],
            None => [self.selected(), self.split]
                .into_iter()
                .flatten()
                .filter_map(|id| self.live.get(&id).map(|l| (l.view.clone(), l.surface.read(cx).bounds())))
                .collect(),
        };
        for (view, surface) in pages {
            if let Some(dropdown) = self.render_field_dropdown(&view, surface, cx) {
                out.push(dropdown);
            }
        }
        out
    }

    /// Logins offered on `view`'s page, best first.
    fn offered_logins(&self, view: ViewId, cx: &App) -> Vec<Offered> {
        let Some(p) = cx.try_global::<Passwords>() else { return Vec::new() };
        let Some(origin) = p.page(view).and_then(|pg| pg.origin.clone()) else { return Vec::new() };
        p.store
            .for_page(&origin)
            .into_iter()
            .map(|(cred, matched)| Offered { cred: cred.clone(), matched })
            .collect()
    }

    /// Where the pill popover goes: under the URL pill, or at the top of
    /// the page while the sidebar is hidden.
    fn pill_popover_origin(&self, window: &Window, cx: &App) -> Point<Pixels> {
        let fullscreen = self.selected_live().is_some_and(|l| l.fullscreen);
        let pill_shown = (self.settings.sidebar_visible || self.sidebar_peek) && !fullscreen;
        let viewport = window.viewport_size();
        let max_x = (f32::from(viewport.width) - POPOVER_WIDTH - 8.).max(8.);
        if pill_shown && let Some(b) = anchor_bounds(crate::ui::site_info::OMNIBOX_ANCHOR, window) {
            return point(px(f32::from(b.left()).min(max_x)), b.bottom() + px(6.));
        }
        let page = match &self.popup {
            Some(popup) => popup.surface.read(cx).bounds(),
            None => self.active_live().map(|l| l.surface.read(cx).bounds()).unwrap_or_default(),
        };
        point(page.left() + px(12.), page.top() + px(12.))
    }

    fn render_pill_popover(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (view, page) = self.passwords_active(cx)?;
        let page = page?;
        let kind = page.popover?;
        let origin = self.pill_popover_origin(window, cx);
        let body = match kind {
            PillPopover::Offer => self.render_offer(view, window, cx)?,
            PillPopover::Logins => self.render_logins_popover(view, cx),
        };
        Some(
            popover_surface(cx)
                .id("password-popover")
                .absolute()
                .left(origin.x)
                .top(origin.y)
                .w(px(POPOVER_WIDTH))
                .occlude()
                .on_mouse_down_out(cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                    // The key button toggles the popover itself.
                    if anchor_bounds(KEY_ANCHOR, window).is_some_and(|b| b.contains(&ev.position)) {
                        return;
                    }
                    this.passwords_dismiss_popover(view, cx);
                }))
                .child(body)
                .into_any_element(),
        )
    }

    /// "Save password?" / "Update password?".
    fn render_offer(&mut self, view: ViewId, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let p = cx.try_global::<Passwords>()?;
        let offer = p.page(view)?.offer.as_ref()?;
        let (serial, kind, username, host) =
            (offer.serial, offer.kind, offer.username.clone(), display_origin(&offer.origin));
        let password = offer.password.clone();
        if self.passwords_ui.offer_input.as_ref().map(|(s, _)| *s) != Some(serial) {
            let input = cx.new(|cx| {
                let mut s = InputState::new(window, cx).placeholder("No username");
                s.set_value(username.clone(), window, cx);
                s
            });
            self.passwords_ui.offer_input = Some((serial, input));
            self.passwords_ui.offer_reveal = false;
        }
        let input = self.passwords_ui.offer_input.as_ref()?.1.clone();
        let reveal = self.passwords_ui.offer_reveal;
        let t = cx.tokens().clone();
        let (title, accept) = match kind {
            OfferKind::Save => ("Save password?", "Save"),
            OfferKind::Update(_) => ("Update password?", "Update"),
        };
        let shown_password: SharedString = if reveal {
            password.expose().to_string().into()
        } else {
            "•".repeat(password.expose().chars().count().clamp(8, 24)).into()
        };
        let label = |text: &'static str| {
            div().text_size(metrics::TEXT_SMALL).text_color(t.muted_foreground).child(text)
        };
        Some(
            v_flex()
                .p(px(14.))
                .gap(px(12.))
                .child(
                    h_flex()
                        .gap(px(10.))
                        .child(
                            div()
                                .size(px(32.))
                                .flex_none()
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .bg(t.primary.opacity(0.14))
                                .child(icon("key-round", px(16.), t.primary)),
                        )
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .child(div().font_weight(FontWeight::SEMIBOLD).text_size(metrics::TEXT_TITLE).child(title))
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(metrics::TEXT_LABEL)
                                        .text_color(t.muted_foreground)
                                        .child(host),
                                ),
                        )
                        .child(
                            icon_button("password-offer-close", "x", px(24.), true, cx)
                                .on_click(cx.listener(move |this, _, _, cx| this.passwords_dismiss_popover(view, cx))),
                        ),
                )
                .child(v_flex().gap(px(4.)).child(label("Username")).child(Input::new(&input)))
                .child(
                    v_flex().gap(px(4.)).child(label("Password")).child(
                        h_flex()
                            .h(px(30.))
                            .pl(px(10.))
                            .pr(px(3.))
                            .gap_2()
                            .rounded(metrics::RADIUS_MD)
                            .bg(t.foreground.opacity(0.05))
                            .border_1()
                            .border_color(t.border.opacity(0.7))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .when(reveal, |s| s.font_family("monospace"))
                                    .child(shown_password),
                            )
                            .child(
                                icon_button("password-offer-reveal", if reveal { "eye-off" } else { "eye" }, px(24.), true, cx)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.passwords_ui.offer_reveal = !this.passwords_ui.offer_reveal;
                                        cx.notify();
                                    })),
                            ),
                    ),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(match kind {
                            OfferKind::Save => text_button("password-never", "Never for This Site", false, cx)
                                .on_click(cx.listener(move |this, _, _, cx| this.passwords_never_for_site(view, cx))),
                            OfferKind::Update(_) => text_button("password-not-now", "Not Now", false, cx)
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    if let Some(page) = Passwords::get(cx).pages.get_mut(&view) {
                                        page.offer = None;
                                        page.popover = None;
                                    }
                                    cx.notify();
                                })),
                        })
                        .child(div().flex_1())
                        .child(text_button("password-accept", accept, true, cx).on_click(cx.listener({
                            let input = input.clone();
                            move |this, _, _, cx| {
                                let username = input.read(cx).value().to_string();
                                this.passwords_accept_offer(view, username, cx);
                            }
                        }))),
                )
                .into_any_element(),
        )
    }

    /// The site's saved logins (key button without a pending offer).
    fn render_logins_popover(&mut self, view: ViewId, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.tokens().clone();
        let logins = self.offered_logins(view, cx);
        let page_origin = cx.try_global::<Passwords>().and_then(|p| p.page(view)).and_then(|pg| pg.origin.clone());
        let host = page_origin.as_deref().map(display_origin).unwrap_or_default();
        v_flex()
            .p(px(6.))
            .child(
                div()
                    .px(px(8.))
                    .pt(px(6.))
                    .pb(px(4.))
                    .text_size(metrics::TEXT_LABEL)
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(t.muted_foreground)
                    .truncate()
                    .child(format!("Saved passwords for {host}")),
            )
            .when(logins.is_empty(), |s| {
                s.child(
                    div()
                        .px(px(8.))
                        .py(px(8.))
                        .text_color(t.muted_foreground)
                        .child("No passwords saved for this site."),
                )
            })
            .children(logins.into_iter().enumerate().map(|(i, o)| self.render_login_row(("pill-login", i), view, o, cx)))
            .child(self.render_manage_row("pill-manage", cx))
            .into_any_element()
    }

    /// A saved login to fill, or the confirmation for a related site's.
    fn render_login_row(&mut self, id: (&'static str, usize), view: ViewId, o: Offered, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.tokens().clone();
        let cred_id = o.cred.id;
        let confirming = cx.try_global::<Passwords>().and_then(|p| p.page(view)).and_then(|pg| pg.confirm) == Some(cred_id);
        let hover = t.foreground.opacity(if t.dark { 0.10 } else { 0.07 });
        if confirming {
            return v_flex()
                .id(id)
                .mx(px(2.))
                .my(px(2.))
                .p(px(10.))
                .gap(px(8.))
                .rounded(px(8.))
                .bg(t.status_warning.opacity(0.10))
                .child(
                    div()
                        .text_size(metrics::TEXT_LABEL)
                        .child(format!("This password was saved for {}. Fill it in here?", o.cred.host())),
                )
                .child(
                    h_flex()
                        .justify_end()
                        .gap_2()
                        .child(text_button(("login-confirm-cancel", id.1), "Cancel", false, cx).on_click(cx.listener(
                            move |_, _, _, cx| {
                                if let Some(page) = Passwords::get(cx).pages.get_mut(&view) {
                                    page.confirm = None;
                                }
                                cx.notify();
                            },
                        )))
                        .child(text_button(("login-confirm-fill", id.1), "Fill", true, cx).on_click(cx.listener(
                            move |this, _, _, cx| this.passwords_fill(view, cred_id, true, cx),
                        ))),
                )
                .into_any_element();
        }
        let name: SharedString = if o.cred.username.is_empty() { "No username".into() } else { o.cred.username.clone().into() };
        let detail = match o.matched {
            OriginMatch::Exact => None,
            OriginMatch::Related => Some(o.cred.host()),
        };
        h_flex()
            .id(id)
            .h(px(ROW_HEIGHT))
            .px(px(8.))
            .gap(px(10.))
            .rounded(px(6.))
            .cursor_pointer()
            .hover(|s| s.bg(hover))
            .child(icon("circle-user-round", px(16.), t.foreground.opacity(0.6)))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .when(o.cred.username.is_empty(), |s| s.text_color(t.muted_foreground))
                            .child(name),
                    )
                    .when_some(detail, |s, d| {
                        s.child(div().truncate().text_size(metrics::TEXT_SMALL).text_color(t.muted_foreground).child(d))
                    }),
            )
            .on_click(cx.listener(move |this, _, _, cx| this.passwords_fill(view, cred_id, false, cx)))
            .into_any_element()
    }

    fn render_manage_row(&mut self, id: &'static str, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.tokens().clone();
        let hover = t.foreground.opacity(if t.dark { 0.10 } else { 0.07 });
        v_flex()
            .child(div().my(px(4.)).mx(px(6.)).h(px(1.)).bg(t.popover_separator()))
            .child(
                h_flex()
                    .id(id)
                    .h(px(32.))
                    .px(px(8.))
                    .gap(px(10.))
                    .rounded(px(6.))
                    .cursor_pointer()
                    .hover(|s| s.bg(hover))
                    .text_color(t.foreground.opacity(0.85))
                    .child(icon("key-round", px(15.), t.foreground.opacity(0.6)))
                    .child("Manage Passwords…")
                    .on_click(cx.listener(|this, _, window, cx| {
                        if let Some((view, _)) = this.passwords_active(cx) {
                            this.passwords_dismiss_popover(view, cx);
                        }
                        for id in [this.selected(), this.split].into_iter().flatten() {
                            if let Some(view) = this.live.get(&id).map(|l| l.view.id())
                                && let Some(page) = Passwords::get(cx).pages.get_mut(&view)
                            {
                                page.dropdown = false;
                            }
                        }
                        this.show_settings(SettingsCategory::Passwords, window, cx);
                    })),
            )
            .into_any_element()
    }

    /// The suggestions dropdown under the focused login field of a page on
    /// screen (drawn in `surface`): its saved logins, or a strong password
    /// for a new-password field.
    fn render_field_dropdown(&mut self, page_view: &WebView, surface: Bounds<Pixels>, cx: &mut Context<Self>) -> Option<AnyElement> {
        let view = page_view.id();
        let p = cx.try_global::<Passwords>()?;
        let page = p.page(view)?;
        if !page.dropdown {
            return None;
        }
        let focus = page.focus?;
        let suggestion = page.suggestion.clone();
        let zoom = mori_core::zoom::factor_for_level(page_view.zoom_level()) as f32;
        let t = cx.tokens().clone();

        let rows: Vec<AnyElement> = if focus.kind == FieldKind::NewPassword {
            let password = suggestion?;
            let hover = t.foreground.opacity(if t.dark { 0.10 } else { 0.07 });
            vec![
                h_flex()
                    .id("password-suggestion")
                    .h(px(46.))
                    .px(px(8.))
                    .gap(px(10.))
                    .rounded(px(6.))
                    .cursor_pointer()
                    .hover(|s| s.bg(hover))
                    .child(icon("wand-sparkles", px(16.), t.primary))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(div().font_weight(FontWeight::MEDIUM).child("Use Strong Password"))
                            .child(
                                div()
                                    .truncate()
                                    .font_family("monospace")
                                    .text_size(metrics::TEXT_SMALL)
                                    .text_color(t.muted_foreground)
                                    .child(password.expose().to_string()),
                            ),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.passwords_use_suggestion(view, cx)))
                    .into_any_element(),
                div()
                    .px(px(8.))
                    .pt(px(4.))
                    .pb(px(6.))
                    .text_size(metrics::TEXT_SMALL)
                    .text_color(t.muted_foreground)
                    .child("Mori offers to save it once you sign up.")
                    .into_any_element(),
            ]
        } else {
            let logins = self.offered_logins(view, cx);
            if logins.is_empty() {
                return None;
            }
            let mut rows: Vec<AnyElement> = logins
                .into_iter()
                .take(6)
                .enumerate()
                .map(|(i, o)| self.render_login_row(("field-login", i), view, o, cx))
                .collect();
            rows.push(self.render_manage_row("field-manage", cx));
            rows
        };

        // The field's box in window coordinates.
        let left = f32::from(surface.left()) + focus.rect.x * zoom;
        let top = f32::from(surface.top()) + focus.rect.y * zoom;
        let bottom = top + focus.rect.h * zoom;
        if bottom < f32::from(surface.top()) || top > f32::from(surface.bottom()) {
            return None;
        }
        let width = (focus.rect.w * zoom).clamp(DROPDOWN_MIN_WIDTH, DROPDOWN_MAX_WIDTH);
        let x = left.min(f32::from(surface.right()) - width - 4.).max(f32::from(surface.left()) + 4.);
        // Below the field, or above it when there's no room below.
        let estimate = rows.len() as f32 * ROW_HEIGHT + 16.;
        let below = bottom + 4.;
        let y = if below + estimate > f32::from(surface.bottom()) && top - estimate - 4. > f32::from(surface.top()) {
            top - estimate - 4.
        } else {
            below
        };
        Some(
            popover_surface(cx)
                .id(("password-dropdown", view.0))
                .absolute()
                .left(px(x))
                .top(px(y))
                .w(px(width))
                .p(px(4.))
                .occlude()
                .on_mouse_down_out(cx.listener(move |_, _, _, cx| {
                    if let Some(page) = Passwords::get(cx).pages.get_mut(&view) {
                        page.dropdown = false;
                        page.confirm = None;
                    }
                    cx.notify();
                }))
                .child(v_flex().children(rows))
                .into_any_element(),
        )
    }
}
