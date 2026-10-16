//! Chrome for windows and profiles: Little Mori's compact header, the
//! private window badge, the "open in another window" card, the window and
//! profile menu entries, and the Default browser, Links and Profiles
//! settings.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    component::input::{Input, InputEvent, InputState},
    *,
};
use mori_core::{
    profiles::ProfileId,
    settings::ExternalLinks,
    util::host_of,
    workspace::{SpaceId, TabId},
};

use crate::{
    browser::{Browser, ToastAction, ToastStyle},
    theme::{ActiveTokens, metrics},
    ui::{
        components::{favicon, h_flex, icon, icon_button, text_button, v_flex},
        launcher::LauncherMode,
        menus::{Entry, item},
        settings::{SettingsCategory, card, choice, field, hint, section},
    },
    windows::{self, WindowKind},
};

/// Height of Little Mori's header.
const LITTLE_HEADER: Pixels = px(42.);

/// Text fields of the profile settings, created on first use.
#[derive(Default)]
pub struct WindowUi {
    new_profile: Option<Entity<InputState>>,
    rename: Option<(ProfileId, Entity<InputState>)>,
    _subscriptions: Vec<Subscription>,
    _rename_events: Option<Subscription>,
}

/// A private window's mark in the sidebar toolbar: the hat-and-glasses
/// glyph in a tinted disc (the space label already says "Private").
pub fn private_badge(b: &Browser, cx: &App) -> Option<AnyElement> {
    if b.kind != WindowKind::Private {
        return None;
    }
    let t = cx.tokens();
    Some(
        div()
            .id("private-badge")
            .size(px(24.))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded_full()
            .bg(t.primary.opacity(0.24))
            .child(icon("hat-glasses", px(14.), t.sidebar_foreground))
            .tooltip(|window, cx| gpui_kit::component::tooltip::Tooltip::new("Private window").build(window, cx))
            .into_any_element(),
    )
}

/// Tab menu entries that move the tab to another window.
pub(crate) fn move_to_window_entries(b: &Browser, tab: TabId) -> Vec<Entry> {
    if b.kind != WindowKind::Normal {
        return Vec::new();
    }
    let mut entries = vec![item("Move to New Window", "app-window", move |_, _, cx| {
        windows::move_tab_to_window(tab, None, cx)
    })];
    for (window, label) in windows::other_windows(b) {
        entries.push(item(format!("Move to Window “{label}”"), "app-window", move |_, _, cx| {
            windows::move_tab_to_window(tab, Some(window), cx)
        }));
    }
    entries
}

/// Space menu entries that pick the space's profile.
pub(crate) fn profile_entries(b: &Browser, space: SpaceId) -> Vec<Entry> {
    if b.kind != WindowKind::Normal {
        return Vec::new();
    }
    let current = windows::profile_of(space);
    let mut entries = vec![Entry::Separator, Entry::Header("Profile".into())];
    entries.push(item(
        "Default",
        if current.is_none() { "check" } else { "user" },
        move |b, _, cx| set_space_profile(b, space, None, cx),
    ));
    for profile in windows::profiles() {
        let id = profile.id;
        entries.push(item(
            profile.name,
            if current == Some(id) { "check" } else { "user" },
            move |b, _, cx| set_space_profile(b, space, Some(id), cx),
        ));
    }
    entries.push(item("New Profile…", "user-plus", move |b, window, cx| {
        let id = windows::create_profile("");
        set_space_profile(b, space, Some(id), cx);
        if b.ws.active_space != space {
            b.switch_space(space, window, cx);
        }
        b.show_settings(SettingsCategory::Profiles, window, cx);
        // After the menu has closed and handed focus back to the page.
        cx.defer_in(window, move |b, window, cx| b.begin_profile_rename(id, window, cx));
    }));
    entries
}

fn set_space_profile(b: &mut Browser, space: SpaceId, profile: Option<ProfileId>, cx: &mut Context<Browser>) {
    if windows::profile_of(space) == profile {
        return;
    }
    windows::assign_profile(space, profile, cx);
    let space_name = b.ws.space(space).map(|s| s.name.clone()).unwrap_or_default();
    let profile_name = profile_name(profile);
    b.toast(format!("{space_name} now uses the {profile_name} profile"), ToastStyle::Success, None, cx);
    cx.notify();
}

fn profile_name(profile: Option<ProfileId>) -> String {
    profile
        .and_then(|id| windows::profiles().into_iter().find(|p| p.id == id))
        .map(|p| format!("“{}”", p.name))
        .unwrap_or_else(|| "default".into())
}

/// The profiles a space can use, as wrapping pills: profile names are the
/// user's and any number of them must fit a narrow window.
fn profile_picker(
    space: SpaceId,
    options: Vec<(String, Option<ProfileId>)>,
    current: Option<ProfileId>,
    cx: &mut Context<Browser>,
) -> Div {
    let t = cx.tokens().clone();
    let selected_bg = if t.dark { white().opacity(0.16) } else { white() };
    h_flex().flex_wrap().gap(px(6.)).children(options.into_iter().enumerate().map(|(i, (label, value))| {
        let active = value == current;
        div()
            .id(("space-profile", i))
            .px_3()
            .h(px(26.))
            .max_w(px(220.))
            .flex()
            .items_center()
            .rounded(metrics::RADIUS_LG)
            .text_size(metrics::TEXT_LABEL)
            .cursor_pointer()
            .border_1()
            .border_color(t.border.opacity(if active { 0.0 } else { 0.7 }))
            .when(active, |s| s.bg(selected_bg).text_color(t.foreground).font_weight(FontWeight::MEDIUM))
            .when(!active, |s| s.text_color(t.foreground.opacity(0.75)).hover(|s| s.bg(t.ghost_hover())))
            .child(div().min_w_0().truncate().child(label))
            .on_click(cx.listener(move |b, _, _, cx| set_space_profile(b, space, value, cx)))
    }))
}

/// Empty header space that moves the window; a double click maximizes.
fn drag_area(id: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .flex_1()
        .h_full()
        .on_mouse_down(MouseButton::Left, |ev: &MouseDownEvent, window, _| {
            if ev.click_count == 2 {
                window.zoom_window();
            } else {
                window.start_window_move();
            }
        })
}

impl Browser {
    /// Little Mori's header: back, the page's address, "Open in <space>"
    /// and close.
    pub(crate) fn render_little_header(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.tokens().clone();
        let record = self.selected().and_then(|id| self.ws.tab(id));
        let url = record.map(|r| r.url.clone()).unwrap_or_default();
        let title = record.map(|r| r.display_title().to_string()).unwrap_or_default();
        let host = host_of(&url);
        let image = host.as_deref().and_then(|h| self.favicons.get(h));
        // The port stays (as in the sidebar's pill).
        let address = Some(mori_core::omnibox::display_host(&url)).filter(|h| !h.is_empty()).unwrap_or_else(|| url.clone());
        let lead = self.security_glyph(self.selected(), &url, t.muted_foreground, cx);
        let can_back = self.selected_live().is_some_and(|l| l.can_go_back);
        let target = windows::little_target_name(cx);
        let open_label = format!("Open in {target}");

        h_flex()
            .id("little-header")
            .h(LITTLE_HEADER)
            .flex_none()
            .px(px(8.))
            .gap(px(6.))
            .child(
                icon_button("little-back", "arrow-left", px(28.), can_back, cx).on_click(cx.listener(
                    |this, _, _, _| {
                        if let Some(v) = this.selected_view() {
                            v.go_back();
                        }
                    },
                )),
            )
            .child(
                h_flex()
                    .id("little-address")
                    .h(px(28.))
                    .min_w_0()
                    .max_w(px(520.))
                    .flex_shrink(1.)
                    .px(px(10.))
                    .gap(px(7.))
                    .rounded(metrics::RADIUS_LG)
                    .bg(t.foreground.opacity(if t.dark { 0.08 } else { 0.05 }))
                    .cursor_pointer()
                    .hover(|s| s.bg(t.ghost_hover()))
                    .child(favicon(image, px(16.), false, cx))
                    .children(lead)
                    .child(
                        div()
                            .flex_none()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(t.sidebar_foreground)
                            .child(address),
                    )
                    .when(!title.is_empty() && Some(&title) != host.as_ref(), |s| {
                        s.child(div().min_w_0().truncate().text_color(t.muted_foreground).child(title))
                    })
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_launcher(LauncherMode::Address, window, cx)
                    })),
            )
            // Saved logins and the save offer (the sidebar's pill has it in
            // normal windows).
            .children(self.password_key_button(cx))
            .child(drag_area("little-drag"))
            .child(
                text_button("little-open", open_label, true, cx)
                    .h(px(28.))
                    .gap(px(6.))
                    .child(icon("maximize-2", px(13.), t.primary_foreground))
                    .on_click(cx.listener(|_, _, _, cx| windows::open_little_in_mori(cx))),
            )
            .child(
                icon_button("little-close", "x", px(28.), true, cx)
                    .on_click(|_, window, _| window.remove_window()),
            )
            .into_any_element()
    }

    /// The selected tab's page is open in another window: say so, with a
    /// way to bring it here or go there.
    pub(crate) fn render_page_elsewhere(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let tab = self.selected()?;
        if self.live.contains_key(&tab) {
            return None;
        }
        let other = windows::window_showing(self, tab, cx)?;
        let t = cx.tokens().clone();
        let title = self.ws.tab(tab).map(|r| r.display_title().to_string()).unwrap_or_default();
        Some(
            v_flex()
                .id("page-elsewhere")
                .size_full()
                .items_center()
                .justify_center()
                .gap_3()
                .child(icon("app-window", px(40.), t.foreground.opacity(0.25)))
                .child(
                    div()
                        .text_size(metrics::TEXT_TITLE)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Open in another window"),
                )
                .child(div().max_w(px(420.)).truncate().text_color(t.muted_foreground).child(title))
                .child(
                    h_flex()
                        .gap_2()
                        .pt_1()
                        .child(text_button("elsewhere-here", "Show Here", true, cx).on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.realize(tab, window, cx);
                                if this.live.contains_key(&tab) {
                                    this.focus_selected_page(window, cx);
                                } else {
                                    // A dialog or prompt keeps it in the other window.
                                    this.toast(
                                        "The page is waiting for an answer in the other window",
                                        ToastStyle::Info,
                                        None,
                                        cx,
                                    );
                                }
                                cx.notify();
                            },
                        )))
                        .child(
                            text_button("elsewhere-go", "Go to Window", false, cx)
                                .on_click(move |_, _, cx| windows::activate(other, cx)),
                        ),
                )
                .into_any_element(),
        )
    }

    /// General settings: default browser and where links from other apps go.
    pub(crate) fn render_browser_settings(&mut self, cx: &mut Context<Self>) -> Div {
        let t = cx.tokens().clone();
        let status = crate::default_browser::status(cx);
        let links = self.settings.external_links;
        let status_text = match status {
            Some(true) => "Mori is your default browser.",
            Some(false) => "Another browser opens links from other apps.",
            None => "Your desktop doesn't say which browser is the default.",
        };
        v_flex()
            .gap(px(22.))
            .child(
                section("Default browser", cx).child(
                    card(cx).child(
                        h_flex()
                            .gap_3()
                            .min_h(px(30.))
                            .child(
                                h_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_2()
                                    .child(icon(
                                        if status == Some(true) { "circle-check" } else { "globe" },
                                        px(16.),
                                        if status == Some(true) { t.primary } else { t.muted_foreground },
                                    ))
                                    .child(div().min_w_0().child(status_text)),
                            )
                            .when(status != Some(true), |s| {
                                s.child(text_button("make-default", "Make Default", true, cx).on_click(
                                    cx.listener(|_, _, _, cx| crate::default_browser::make_default(cx)),
                                ))
                            }),
                    ),
                ),
            )
            .child(
                section("Links from other apps", cx).child(
                    card(cx)
                        .child(field(
                            "Open links in",
                            choice(
                                "external-links",
                                vec![
                                    ("Little Mori".into(), ExternalLinks::LittleWindow),
                                    ("Current space".into(), ExternalLinks::CurrentSpace),
                                ],
                                links,
                                |b, v, _, _| b.settings.external_links = v,
                                cx,
                            ),
                            cx,
                        ))
                        .child(hint(
                            match links {
                                ExternalLinks::LittleWindow => {
                                    "A link from another app opens in a small window; “Open in Mori” keeps it as a tab."
                                }
                                ExternalLinks::CurrentSpace => {
                                    "A link from another app opens as a tab in the space you used last."
                                }
                            },
                            cx,
                        )),
                ),
            )
    }

    /// The Profiles settings page: the current space's profile and the
    /// list of profiles.
    pub(crate) fn render_profile_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = cx.tokens().clone();
        let new_input = self.profile_name_input(window, cx);
        let space = self.ws.active_space;
        let space_name = self.ws.active().name.clone();
        let current = windows::profile_of(space);
        let profiles = windows::profiles();
        let renaming = self.window_ui.rename.clone();

        let mut options: Vec<(String, Option<ProfileId>)> = vec![("Default".into(), None)];
        options.extend(profiles.iter().map(|p| (p.name.clone(), Some(p.id))));

        let rows: Vec<AnyElement> = profiles
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let id = p.id;
                let used = windows::spaces_using(id);
                let usage = match used {
                    0 => "Not used by a space".to_string(),
                    1 => "Used by 1 space".to_string(),
                    n => format!("Used by {n} spaces"),
                };
                let name: AnyElement = match &renaming {
                    Some((rid, input)) if *rid == id => {
                        div().flex_1().min_w_0().child(Input::new(input)).into_any_element()
                    }
                    _ => v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(div().truncate().child(p.name.clone()))
                        .child(hint(usage, cx))
                        .into_any_element(),
                };
                h_flex()
                    .gap_2()
                    .min_h(px(38.))
                    .child(icon("user", px(16.), t.muted_foreground))
                    .child(name)
                    .child(
                        icon_button(("profile-rename", i), "pencil", px(26.), true, cx).on_click(cx.listener(
                            move |this, _, window, cx| this.begin_profile_rename(id, window, cx),
                        )),
                    )
                    .child(
                        icon_button(("profile-delete", i), "trash", px(26.), true, cx).on_click(cx.listener(
                            move |this, _, _, cx| {
                                if let Some(deleted) = windows::delete_profile(id, cx) {
                                    let name = deleted.profile.name.clone();
                                    this.toast(
                                        format!("Deleted profile “{name}”"),
                                        ToastStyle::Info,
                                        Some(("Undo".into(), ToastAction::UndoDeleteProfile(Box::new(deleted)))),
                                        cx,
                                    );
                                }
                                cx.notify();
                            },
                        )),
                    )
                    .into_any_element()
            })
            .collect();

        let normal = self.kind == WindowKind::Normal;
        v_flex()
            .gap(px(22.))
            .when(normal, |page| {
                page.child(
                    section(&format!("Profile for “{space_name}”"), cx).child(
                        card(cx)
                            .child(field("Profile", profile_picker(space, options, current, cx), cx))
                            .child(hint(
                                "Each profile keeps its own cookies, logins and site data, so you can be signed in \
                                 to different accounts in different spaces. Pages reload when the profile changes.",
                                cx,
                            )),
                    ),
                )
            })
            .when(!normal, |page| {
                page.child(hint("Private and Little Mori windows don't use profiles.", cx))
            })
            .child(
                section("Profiles", cx).child(
                    card(cx)
                        .when(rows.is_empty(), |c| {
                            c.child(hint("Every space uses the default profile.", cx))
                        })
                        .children(rows)
                        .child(
                            h_flex()
                                .gap_2()
                                .child(div().flex_1().min_w_0().child(Input::new(&new_input)))
                                .child(text_button("profile-create", "Create Profile", true, cx).on_click(
                                    cx.listener(|this, _, window, cx| this.create_profile_from_input(window, cx)),
                                )),
                        )
                        .child(hint(
                            "A deleted profile's cookies and site data are removed the next time Mori starts.",
                            cx,
                        )),
                ),
            )
    }

    fn profile_name_input(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
        if let Some(input) = &self.window_ui.new_profile {
            return input.clone();
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("New profile name"));
        let sub = cx.subscribe_in(&input, window, |this, _, ev: &InputEvent, window, cx| {
            if matches!(ev, InputEvent::PressEnter { .. }) {
                this.create_profile_from_input(window, cx);
            }
        });
        self.window_ui._subscriptions.push(sub);
        self.window_ui.new_profile = Some(input.clone());
        input
    }

    fn create_profile_from_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(input) = self.window_ui.new_profile.clone() else { return };
        let name = input.read(cx).value().to_string();
        let id = windows::create_profile(&name);
        input.update(cx, |s, cx| s.set_value("", window, cx));
        let name = windows::profiles().into_iter().find(|p| p.id == id).map(|p| p.name).unwrap_or_default();
        self.toast(format!("Created profile “{name}”"), ToastStyle::Success, None, cx);
        cx.notify();
    }

    /// Rename a profile inline in the Profiles settings.
    pub(crate) fn begin_profile_rename(&mut self, id: ProfileId, window: &mut Window, cx: &mut Context<Self>) {
        let current = windows::profiles().into_iter().find(|p| p.id == id).map(|p| p.name).unwrap_or_default();
        let input = cx.new(|cx| {
            let mut state = InputState::new(window, cx);
            state.set_value(current, window, cx);
            state
        });
        input.update(cx, |s, cx| {
            s.focus(window, cx);
            s.select_all(window, cx);
        });
        let sub = cx.subscribe_in(&input, window, move |this, input, ev: &InputEvent, window, cx| {
            if matches!(ev, InputEvent::PressEnter { .. } | InputEvent::Blur)
                && this.window_ui.rename.as_ref().is_some_and(|(rid, _)| *rid == id)
            {
                let value = input.read(cx).value().to_string();
                windows::rename_profile(id, &value);
                this.window_ui.rename = None;
                this.window_ui._rename_events = None;
                // The field that had the keys goes away with the edit.
                if matches!(ev, InputEvent::PressEnter { .. }) {
                    this.focus_selected_page(window, cx);
                }
                cx.notify();
            }
        });
        self.window_ui._rename_events = Some(sub);
        self.window_ui.rename = Some((id, input));
        cx.notify();
    }
}
