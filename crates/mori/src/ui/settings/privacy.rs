//! The Privacy page of the settings: the content blocker, tracking
//! protection, clearing browsing data, site permissions, and cookies.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use mori_core::{
    content_blocker::LISTS,
    privacy::{ClearOptions, TimeRange},
    site_permissions::Decision,
};

use super::{card, choice, field, hint, section, toggle};
use crate::{
    browser::Browser,
    theme::{ActiveTokens, metrics},
    ui::components::{h_flex, icon, icon_button, relative_time, text_button, v_flex},
};

/// Label and icon for a remembered permission name.
fn permission_label(key: &str) -> (&'static str, &'static str) {
    match key {
        "camera" => ("Camera", "camera"),
        "microphone" => ("Microphone", "mic"),
        "geolocation" => ("Location", "map-pin"),
        "notifications" => ("Notifications", "bell"),
        "clipboard" => ("Clipboard", "clipboard"),
        "midi" => ("MIDI", "piano"),
        "sensors" => ("Sensors", "activity"),
        "screen-capture" => ("Screen sharing", "monitor"),
        "storage" => ("Cross-site cookies", "cookie"),
        "window-management" => ("Windows", "app-window"),
        "local-fonts" => ("Fonts", "type"),
        "idle-detection" => ("Idle detection", "timer"),
        _ => ("Other", "shield"),
    }
}

/// A checkbox row for "Clear browsing data".
fn check_row(
    id: &'static str,
    label: &'static str,
    detail: Option<&'static str>,
    on: bool,
    set: impl Fn(&mut ClearOptions, bool) + 'static,
    cx: &mut Context<Browser>,
) -> Stateful<Div> {
    let t = cx.tokens().clone();
    h_flex()
        .id(id)
        .gap_3()
        .min_h(px(30.))
        .cursor_pointer()
        .child(
            div()
                .size(px(16.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.))
                .border_1()
                .border_color(if on { t.primary } else { t.border })
                .when(on, |s| s.bg(t.primary).child(icon("check", px(12.), t.primary_foreground))),
        )
        .child(
            v_flex()
                .child(div().text_color(t.foreground.opacity(0.85)).child(label))
                .when_some(detail, |s, d| s.child(hint(d, cx))),
        )
        .on_click(cx.listener(move |this, _, _, cx| {
            set(&mut this.privacy.clear, !on);
            cx.notify();
        }))
}

impl Browser {
    pub(super) fn render_privacy_settings(&mut self, cx: &mut Context<Self>) -> Div {
        let t = cx.tokens().clone();
        let s = self.settings.clone();
        let status = self.lists_status(cx);
        let now = mori_core::paths::now();

        // ---- Content blocker
        let list_names: Vec<&str> = LISTS.iter().map(|l| l.name).collect();
        let status_text = if status.updating {
            "Updating filter lists…".to_string()
        } else if let Some(rules) = status.rules {
            let when = status.updated_at.map(|t| format!(" · Updated {}", relative_time(t, now))).unwrap_or_default();
            format!("{} · {} rules{when}", list_names.join(" and "), group_digits(rules))
        } else {
            format!("Using Mori's built-in list until {} have downloaded.", list_names.join(" and "))
        };
        let exceptions: Vec<AnyElement> = self
            .privacy
            .exceptions
            .sites
            .iter()
            .enumerate()
            .map(|(i, site)| {
                let site = site.clone();
                h_flex()
                    .gap_2()
                    .h(px(30.))
                    .child(icon("shield-off", px(14.), t.muted_foreground))
                    .child(div().flex_1().min_w_0().truncate().child(site.clone()))
                    .child(
                        icon_button(("blocker-exception", i), "x", px(24.), true, cx)
                            .on_click(cx.listener(move |this, _, _, cx| this.remove_blocking_exception(&site, cx))),
                    )
                    .into_any_element()
            })
            .collect();
        let blocker = section("Content blocker", cx).child(
            card(cx)
                .child(field(
                    "Block ads and trackers",
                    toggle("blocker-toggle", s.content_blocker, |b, on| b.settings.content_blocker = on, cx)
                        .on_click(cx.listener(|this, _, _, cx| this.sync_content_blocker(cx))),
                    cx,
                ))
                .child(
                    h_flex()
                        .gap_3()
                        .child(div().flex_1().min_w_0().child(hint(status_text, cx)))
                        .child(
                            text_button("blocker-update", "Update Lists", false, cx)
                                .flex_none()
                                .when(status.updating, |b| b.opacity(0.5))
                                .on_click(cx.listener(|this, _, _, cx| this.update_filter_lists(cx))),
                        ),
                )
                .when_some(status.last_error.clone().filter(|_| !status.updating), |c, e| {
                    c.child(
                        div()
                            .text_size(metrics::TEXT_SMALL)
                            .text_color(t.status_warning)
                            .child(format!("The last update failed: {e}")),
                    )
                })
                .when(!exceptions.is_empty(), |c| {
                    c.child(
                        div()
                            .pt_1()
                            .text_size(metrics::TEXT_SMALL)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(t.muted_foreground)
                            .child("Sites where blocking is off"),
                    )
                    .children(exceptions)
                })
                .when(self.privacy.exceptions.sites.is_empty(), |c| {
                    c.child(hint("Turn blocking off for a site from the site menu in the address bar.", cx))
                }),
        );

        // ---- Tracking
        let third_party = mori_engine::privacy::can_block_third_party_cookies();
        let tracking = section("Tracking", cx).child(
            card(cx)
                .child(field(
                    "Send Do Not Track",
                    toggle("dnt-toggle", s.do_not_track, |b, on| {
                        b.settings.do_not_track = on;
                        b.apply_tracking_settings();
                    }, cx),
                    cx,
                ))
                .child(hint(
                    "Asks sites not to track you, with the Do Not Track and Global Privacy Control signals. Pages loaded before keep the old setting until reloaded.",
                    cx,
                ))
                .when(third_party, |c| {
                    c.child(field(
                        "Third-party cookies",
                        choice(
                            "third-party",
                            vec![("Allow".into(), false), ("Block".into(), true)],
                            s.block_third_party_cookies,
                            |b, on, _, _| {
                                b.settings.block_third_party_cookies = on;
                                b.apply_tracking_settings();
                            },
                            cx,
                        ),
                        cx,
                    ))
                    .child(hint("Sites embedded in a page can't use their cookies. Some sign-in buttons and comment widgets may stop working.", cx))
                }),
        );

        // ---- Clear browsing data
        let clear = self.privacy.clear;
        let clear_card = section("Clear browsing data", cx).child(
            card(cx)
                .child(field(
                    "Time range",
                    choice(
                        "clear-range",
                        TimeRange::ALL.iter().map(|r| (r.label().to_string(), *r)).collect(),
                        clear.range,
                        |b, v, _, _| b.privacy.clear.range = v,
                        cx,
                    ),
                    cx,
                ))
                .child(check_row("clear-history", "Browsing history", None, clear.history, |o, v| o.history = v, cx))
                .child(check_row(
                    "clear-cookies",
                    "Cookies and site data",
                    Some("Signs you out of most sites."),
                    clear.cookies,
                    |o, v| o.cookies = v,
                    cx,
                ))
                .child(check_row(
                    "clear-cache",
                    "Cached images and files",
                    (clear.range != TimeRange::AllTime).then_some("Chromium clears its cache for all time."),
                    clear.cache,
                    |o, v| o.cache = v,
                    cx,
                ))
                .child(check_row("clear-downloads", "Downloads list", Some("Files stay on disk."), clear.downloads, |o, v| o.downloads = v, cx))
                .child(
                    h_flex().justify_end().child(
                        text_button("clear-data", "Clear Data", true, cx)
                            .when(!clear.any(), |b| b.opacity(0.5))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let opts = this.privacy.clear;
                                this.clear_browsing_data(opts, cx);
                            })),
                    ),
                ),
        );

        // ---- Site permissions
        let sites: Vec<(String, Vec<(String, Decision)>)> =
            self.site_permissions.sites().map(|(o, d)| (o.to_string(), d)).collect();
        let site_rows: Vec<AnyElement> = sites
            .into_iter()
            .enumerate()
            .map(|(i, (origin, decisions))| {
                let host = mori_core::omnibox::display_host(&origin);
                let chips = decisions.into_iter().enumerate().map(|(j, (key, decision))| {
                    let (label, glyph) = permission_label(&key);
                    let allowed = decision == Decision::Allow;
                    let color = if allowed { t.primary } else { t.status_warning };
                    let (flip_origin, flip_key) = (origin.clone(), key.clone());
                    let (revoke_origin, revoke_key) = (origin.clone(), key.clone());
                    h_flex()
                        .id(("perm-chip", i * 100 + j))
                        .h(px(24.))
                        .pl(px(8.))
                        .pr(px(2.))
                        .gap(px(5.))
                        .rounded_full()
                        .bg(color.opacity(0.12))
                        .text_size(metrics::TEXT_LABEL)
                        .cursor_pointer()
                        .child(icon(glyph, px(12.), color))
                        .child(format!("{label}: {}", if allowed { "Allowed" } else { "Blocked" }))
                        .child(
                            div()
                                .id(("perm-revoke", i * 100 + j))
                                .size(px(20.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .hover(|s| s.bg(t.ghost_hover()))
                                .child(icon("x", px(11.), t.muted_foreground))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.revoke_permission(&revoke_origin, &revoke_key, cx);
                                })),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| this.flip_permission(&flip_origin, &flip_key, cx)))
                });
                let reset_origin = origin.clone();
                v_flex()
                    .gap_1p5()
                    .py_1()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(div().flex_1().min_w_0().truncate().font_weight(FontWeight::MEDIUM).child(host))
                            .child(
                                icon_button(("perm-reset", i), "rotate-ccw", px(24.), true, cx)
                                    .tooltip(|window, cx| {
                                        gpui_kit::component::tooltip::Tooltip::new("Reset permissions").build(window, cx)
                                    })
                                    .on_click(cx.listener(move |this, _, _, cx| this.reset_site_permissions(&reset_origin, cx))),
                            ),
                    )
                    .child(h_flex().flex_wrap().gap(px(6.)).children(chips))
                    .into_any_element()
            })
            .collect();
        let permissions = section("Site permissions", cx).child(
            card(cx)
                .when(site_rows.is_empty(), |c| {
                    c.child(hint("Sites you allow or block from using your camera, location, notifications and more show up here.", cx))
                })
                .when(!site_rows.is_empty(), |c| c.child(hint("Click a permission to switch it, or × to ask again next time.", cx)))
                .children(site_rows),
        );

        // ---- Cookies and site data
        let cookie_body: AnyElement = match &self.privacy.cookie_sites {
            _ if self.privacy.loading_cookies && self.privacy.cookie_sites.is_none() => {
                hint("Reading cookies…", cx).into_any_element()
            }
            None => h_flex()
                .gap_3()
                .child(div().flex_1().child(hint("See which sites keep cookies and data on this computer.", cx)))
                .child(
                    text_button("cookies-show", "Show Sites", false, cx)
                        .on_click(cx.listener(|this, _, _, cx| this.load_cookie_sites(cx))),
                )
                .into_any_element(),
            Some(list) if list.is_empty() => hint("No site keeps cookies right now.", cx).into_any_element(),
            Some(list) => {
                let total: usize = list.iter().map(|(_, n)| n).sum();
                // Removing every cookie signs the user out everywhere: ask
                // once more.
                let header = if self.privacy.confirm_remove_cookies {
                    h_flex()
                        .gap_2()
                        .child(div().flex_1().child(hint("Remove all cookies and site data? You'll be signed out of most sites.", cx)))
                        .child(text_button("cookies-remove-cancel", "Cancel", false, cx).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.privacy.confirm_remove_cookies = false;
                                cx.notify();
                            },
                        )))
                        .child(text_button("cookies-remove-confirm", "Remove All", true, cx).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.privacy.confirm_remove_cookies = false;
                                let opts = ClearOptions {
                                    history: false,
                                    cookies: true,
                                    cache: false,
                                    downloads: false,
                                    range: TimeRange::AllTime,
                                };
                                this.clear_browsing_data(opts, cx);
                            },
                        )))
                } else {
                    h_flex()
                        .gap_3()
                        .child(div().flex_1().child(hint(format!("{} · {}", count_label(list.len(), "site", "sites"), count_label(total, "cookie", "cookies")), cx)))
                        .child(text_button("cookies-remove-all", "Remove All", false, cx).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.privacy.confirm_remove_cookies = true;
                                cx.notify();
                            },
                        )))
                };
                v_flex()
                    .gap_1()
                    .child(header)
                    .child(
                        div().id("cookie-sites").max_h(px(320.)).overflow_y_scroll().children(list.iter().enumerate().map(
                            |(i, (site, count))| {
                                let site_owned = site.clone();
                                h_flex()
                                    .gap_2()
                                    .h(px(30.))
                                    .child(icon("cookie", px(14.), t.muted_foreground))
                                    .child(div().flex_1().min_w_0().truncate().child(site.clone()))
                                    .child(
                                        div()
                                            .text_size(metrics::TEXT_LABEL)
                                            .text_color(t.muted_foreground)
                                            .child(count_label(*count, "cookie", "cookies")),
                                    )
                                    .child(icon_button(("cookie-del", i), "trash", px(24.), true, cx).on_click(cx.listener(
                                        move |this, _, _, cx| this.remove_cookie_site(&site_owned, cx),
                                    )))
                            },
                        )),
                    )
                    .into_any_element()
            }
        };
        let cookies = section("Cookies and site data", cx).child(card(cx).child(cookie_body));

        v_flex()
            .gap(px(22.))
            .child(blocker)
            .child(tracking)
            .child(clear_card)
            .child(permissions)
            .child(cookies)
    }
}

/// `1 cookie`, `3 cookies`.
fn count_label(n: usize, one: &str, many: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {many}") }
}

/// `112948` → `112,948`.
fn group_digits(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{count_label, group_digits};

    #[test]
    fn counts() {
        assert_eq!(count_label(1, "site", "sites"), "1 site");
        assert_eq!(count_label(0, "site", "sites"), "0 sites");
        assert_eq!(count_label(12, "cookie", "cookies"), "12 cookies");
    }

    #[test]
    fn groups_digits() {
        assert_eq!(group_digits(7), "7");
        assert_eq!(group_digits(1234), "1,234");
        assert_eq!(group_digits(112948), "112,948");
        assert_eq!(group_digits(1000000), "1,000,000");
    }
}
