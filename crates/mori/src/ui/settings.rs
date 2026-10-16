//! Settings, rendered as a page inside the web card.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    component::input::{Input, InputEvent, InputState},
    *,
};
use mori_core::{
    settings::{ARCHIVE_OPTIONS, Appearance, NewTabBehavior, SLEEP_OPTIONS, SearchEngine, SidebarPosition},
    theme::{GradientTheme, Rgba, SOLID_SWATCHES, presets},
};

use crate::{
    browser::{Browser, ToastStyle},
    theme::{ActiveTokens, metrics},
    ui::components::{h_flex, icon, icon_button, text_button, v_flex},
};

/// Text fields on the settings page, created on first open.
pub struct SettingsInputs {
    homepage: Entity<InputState>,
    search_template: Entity<InputState>,
    route_pattern: Entity<InputState>,
}

fn section(title: &str, cx: &App) -> Div {
    let t = cx.tokens();
    v_flex()
        .gap_2()
        .child(
            div()
                .text_size(metrics::TEXT_SMALL)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.muted_foreground)
                .child(title.to_uppercase()),
        )
}

fn card(cx: &App) -> Div {
    let t = cx.tokens();
    v_flex()
        .gap_3()
        .p_4()
        .rounded(metrics::RADIUS_POPOVER)
        .bg(t.background.opacity(0.6))
        .border_1()
        .border_color(t.border.opacity(0.6))
}

fn field(label: &str, control: impl IntoElement, cx: &App) -> Div {
    let t = cx.tokens();
    h_flex()
        .gap_3()
        .min_h(px(30.))
        .child(
            div()
                .w(px(140.))
                .flex_none()
                .text_color(t.foreground.opacity(0.85))
                .child(label.to_string()),
        )
        .child(div().flex_1().min_w_0().child(control))
}

impl Browser {
    fn settings_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) -> &SettingsInputs {
        if self.settings_inputs.is_none() {
            let make = |value: String, placeholder: &'static str, window: &mut Window, cx: &mut Context<Self>| {
                cx.new(|cx| {
                    let mut s = InputState::new(window, cx).placeholder(placeholder);
                    s.set_value(value, window, cx);
                    s
                })
            };
            let homepage = make(self.settings.homepage_url.clone(), "mori://newtab", window, cx);
            let search_template = make(
                self.settings.custom_search_template.clone(),
                "https://example.com/search?q={query}",
                window,
                cx,
            );
            let route_pattern = make(String::new(), "example.com", window, cx);
            let s1 = cx.subscribe(&homepage, |this, input, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::Change) {
                    this.settings.homepage_url = input.read(cx).value().to_string();
                    this.schedule_save(cx);
                }
            });
            let s2 = cx.subscribe(&search_template, |this, input, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::Change) {
                    this.settings.custom_search_template = input.read(cx).value().to_string();
                    this.schedule_save(cx);
                }
            });
            let s3 = cx.subscribe(&route_pattern, |this, input, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::PressEnter { .. }) {
                    let value = input.read(cx).value().to_string();
                    this.add_route(&value, cx);
                }
            });
            self.push_subscription(s1);
            self.push_subscription(s2);
            self.push_subscription(s3);
            self.settings_inputs = Some(SettingsInputs {
                homepage,
                search_template,
                route_pattern,
            });
        }
        self.settings_inputs.as_ref().expect("just created")
    }

    fn add_route(&mut self, pattern: &str, cx: &mut Context<Self>) {
        let space = self.ws.active_space;
        let name = self.ws.active().name.clone();
        match self.routes.add(pattern, space) {
            Some(r) => {
                let p = r.pattern.clone();
                self.toast(format!("{p} now opens in {name}"), ToastStyle::Success, None, cx)
            }
            None => self.toast("Enter a domain like example.com", ToastStyle::Warning, None, cx),
        }
        self.schedule_save(cx);
        cx.notify();
    }

    fn set_space_theme(&mut self, theme: GradientTheme, window: &mut Window, cx: &mut Context<Self>) {
        self.ws.active_mut().theme = theme;
        self.apply_theme(Some(window), cx);
        self.schedule_save(cx);
        cx.notify();
    }

    pub fn render_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.tokens().clone();
        let inputs = self.settings_inputs(window, cx);
        let (homepage, search_template, route_pattern) = (
            inputs.homepage.clone(),
            inputs.search_template.clone(),
            inputs.route_pattern.clone(),
        );
        let s = self.settings.clone();

        // Segmented control over a list of (label, value).
        fn segmented<T: Copy + PartialEq + 'static>(
            id: &'static str,
            options: Vec<(String, T)>,
            current: T,
            on_pick: impl Fn(&mut Browser, T, &mut Window, &mut Context<Browser>) + Clone + 'static,
            cx: &mut Context<Browser>,
        ) -> Div {
            let t = cx.tokens().clone();
            h_flex()
                .gap_0p5()
                .p_0p5()
                .rounded(metrics::RADIUS_MD)
                .bg(t.foreground.opacity(0.06))
                .children(options.into_iter().enumerate().map(|(i, (label, value))| {
                    let active = value == current;
                    let on_pick = on_pick.clone();
                    div()
                        .id((id, i))
                        .px_3()
                        .h(px(26.))
                        .flex()
                        .items_center()
                        .rounded(metrics::RADIUS_MD)
                        .text_size(metrics::TEXT_LABEL)
                        .cursor_pointer()
                        .when(active, |s| s.bg(t.primary).text_color(t.primary_foreground))
                        .when(!active, |s| s.hover(|s| s.bg(t.ghost_hover())))
                        .child(label)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            on_pick(this, value, window, cx);
                            this.schedule_save(cx);
                            cx.notify();
                        }))
                }))
        }

        fn toggle(
            id: impl Into<ElementId>,
            on: bool,
            on_change: impl Fn(&mut Browser, bool) + 'static,
            cx: &mut Context<Browser>,
        ) -> Stateful<Div> {
            let t = cx.tokens().clone();
            div()
                .id(id)
                .w(px(36.))
                .h(px(20.))
                .p(px(2.))
                .rounded_full()
                .cursor_pointer()
                .bg(if on { t.primary } else { t.foreground.opacity(0.15) })
                .child(
                    div()
                        .size(px(16.))
                        .rounded_full()
                        .bg(white())
                        .shadow_sm()
                        .when(on, |s| s.ml(px(16.))),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    on_change(this, !on);
                    this.schedule_save(cx);
                    cx.notify();
                }))
        }

        let space_theme = self.ws.active().theme.clone();
        let preset_rows: Vec<AnyElement> = std::iter::once(None)
            .chain(presets().into_iter().map(Some))
            .enumerate()
            .map(|(i, preset)| {
                let (name, subtitle, colors, theme) = match &preset {
                    None => ("None".to_string(), "Plain chrome".to_string(), vec![], GradientTheme::none()),
                    Some(p) => (
                        p.name.to_string(),
                        p.subtitle.to_string(),
                        p.colors.to_vec(),
                        p.theme(),
                    ),
                };
                let active = match &preset {
                    None => space_theme.is_none(),
                    Some(p) => space_theme.preset_id.as_deref() == Some(p.id),
                };
                let swatch = if colors.is_empty() {
                    div().size_full().bg(t.sidebar).into_any_element()
                } else {
                    h_flex()
                        .size_full()
                        .children(colors.iter().map(|c| div().flex_1().h_full().bg(rgb(*c))))
                        .into_any_element()
                };
                h_flex()
                    .id(("preset", i))
                    .gap_3()
                    .p_1()
                    .rounded(metrics::RADIUS_LG)
                    .cursor_pointer()
                    .when(active, |s| s.bg(t.primary.opacity(0.14)))
                    .hover(|s| s.bg(t.ghost_hover()))
                    .child(
                        div()
                            .w(px(60.))
                            .h(px(36.))
                            .rounded(metrics::RADIUS_LG)
                            .overflow_hidden()
                            .border_1()
                            .border_color(t.border.opacity(0.6))
                            .child(swatch),
                    )
                    .child(
                        v_flex()
                            .child(div().font_weight(FontWeight::MEDIUM).child(name))
                            .child(
                                div()
                                    .text_size(metrics::TEXT_SMALL)
                                    .text_color(t.muted_foreground)
                                    .child(subtitle),
                            ),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.set_space_theme(theme.clone(), window, cx)
                    }))
                    .into_any_element()
            })
            .collect();

        let swatches: Vec<AnyElement> = SOLID_SWATCHES
            .iter()
            .enumerate()
            .map(|(i, (name, hex))| {
                let hex = *hex;
                let active = space_theme.solid_hex() == Some(Rgba::hex(hex).to_hex_string().as_str());
                div()
                    .id(("swatch", i))
                    .size(px(24.))
                    .rounded_full()
                    .bg(rgb(hex))
                    .cursor_pointer()
                    .border_2()
                    .border_color(if active { t.foreground } else { transparent_black() })
                    .tooltip({
                        let name: SharedString = (*name).into();
                        move |window, cx| gpui_kit::component::tooltip::Tooltip::new(name.clone()).build(window, cx)
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.set_space_theme(GradientTheme::solid(Rgba::hex(hex)), window, cx)
                    }))
                    .into_any_element()
            })
            .collect();

        let has_theme = !space_theme.is_none();
        let opacity_steps: Vec<(String, f32)> =
            [0.3, 0.45, 0.6, 0.75, 0.9].iter().map(|v| (format!("{:.0}%", v * 100.0), *v)).collect();
        let grain_steps: Vec<(String, f32)> = [
            ("Off", 0.0),
            ("Light", 0.25),
            ("Medium", 0.5),
            ("Heavy", 1.0),
        ]
        .iter()
        .map(|(l, v)| (l.to_string(), *v))
        .collect();
        let current_opacity = opacity_steps
            .iter()
            .map(|(_, v)| *v)
            .min_by(|a, b| (a - space_theme.opacity).abs().total_cmp(&(b - space_theme.opacity).abs()))
            .unwrap_or(0.6);
        let current_grain = grain_steps
            .iter()
            .map(|(_, v)| *v)
            .min_by(|a, b| (a - space_theme.texture).abs().total_cmp(&(b - space_theme.texture).abs()))
            .unwrap_or(0.0);

        let space_names: std::collections::HashMap<_, _> =
            self.ws.spaces.iter().map(|s| (s.id, s.name.clone())).collect();
        let routes: Vec<AnyElement> = self
            .routes
            .list()
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let id = r.id;
                let enabled = r.enabled;
                h_flex()
                    .gap_2()
                    .h(px(30.))
                    .child(div().flex_1().child(r.pattern.clone()))
                    .child(
                        div()
                            .text_color(t.muted_foreground)
                            .child(format!("→ {}", space_names.get(&r.context_id).cloned().unwrap_or_default())),
                    )
                    .child(toggle(
                        ("route-toggle", i),
                        enabled,
                        move |b, on| {
                            b.routes.set_enabled(id, on);
                        },
                        cx,
                    ))
                    .child(icon_button(("route-del", i), "trash-2", px(26.), true, cx).on_click(cx.listener(
                        move |this, _, _, cx| {
                            this.routes.remove(id);
                            this.schedule_save(cx);
                            cx.notify();
                        },
                    )))
                    .into_any_element()
            })
            .collect();

        let boosts: Vec<AnyElement> = self
            .boosts
            .list()
            .iter()
            .enumerate()
            .map(|(i, b)| {
                let (id, host) = (b.id, b.host.clone());
                let summary = format!(
                    "{} hidden · {} CSS · {} JS",
                    b.zapped_selectors.len(),
                    if b.css.trim().is_empty() { "no" } else { "custom" },
                    if b.js.trim().is_empty() { "no" } else { "custom" },
                );
                h_flex()
                    .gap_2()
                    .h(px(30.))
                    .child(v_flex().flex_1().child(host.clone()).child(
                        div().text_size(metrics::TEXT_SMALL).text_color(t.muted_foreground).child(summary),
                    ))
                    .child(text_button(("boost-visit", i), "Visit", false, cx).on_click(cx.listener(
                        move |this, _, window, cx| {
                            this.set_settings_open(false, window, cx);
                            this.open_url(&format!("https://{host}"), true, window, cx);
                        },
                    )))
                    .child(icon_button(("boost-del", i), "trash-2", px(26.), true, cx).on_click(cx.listener(
                        move |this, _, _, cx| {
                            this.boosts.remove(id);
                            this.schedule_save(cx);
                            cx.notify();
                        },
                    )))
                    .into_any_element()
            })
            .collect();

        v_flex()
            .size_full()
            .child(
                h_flex()
                    .h(px(56.))
                    .flex_none()
                    .px_4()
                    .gap_2()
                    .border_b_1()
                    .border_color(t.border.opacity(0.6))
                    .child(icon_button("settings-back", "arrow-left", px(28.), true, cx).on_click(
                        cx.listener(|this, _, window, cx| this.set_settings_open(false, window, cx)),
                    ))
                    .child(
                        div()
                            .flex_1()
                            .text_size(metrics::TEXT_TITLE)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Settings"),
                    )
                    .child(text_button("settings-done", "Done", true, cx).on_click(cx.listener(
                        |this, _, window, cx| this.set_settings_open(false, window, cx),
                    ))),
            )
            .child(
                div().id("settings-scroll").flex_1().overflow_y_scroll().child(
                    v_flex()
                        .w(px(560.))
                        .mx_auto()
                        .py(px(24.))
                        .gap(px(22.))
                        .child(section("General", cx).child(
                            card(cx)
                                .child(field("Homepage", Input::new(&homepage), cx))
                                .child(field(
                                    "New tab opens",
                                    segmented(
                                        "new-tab",
                                        vec![
                                            ("Homepage".into(), NewTabBehavior::Homepage),
                                            ("Blank page".into(), NewTabBehavior::Blank),
                                        ],
                                        s.new_tab_behavior,
                                        |b, v, _, _| b.settings.new_tab_behavior = v,
                                        cx,
                                    ),
                                    cx,
                                )),
                        ))
                        .child(section("Search", cx).child(
                            card(cx)
                                .child(field(
                                    "Search engine",
                                    segmented(
                                        "engine",
                                        SearchEngine::ALL.iter().map(|e| (e.label().to_string(), *e)).collect(),
                                        s.search_engine,
                                        |b, v, _, _| b.settings.search_engine = v,
                                        cx,
                                    ),
                                    cx,
                                ))
                                .when(s.search_engine == SearchEngine::Custom, |c| {
                                    c.child(field("Custom URL", Input::new(&search_template), cx)).child(
                                        div()
                                            .text_size(metrics::TEXT_SMALL)
                                            .text_color(t.muted_foreground)
                                            .child("Use {query} where the search terms go."),
                                    )
                                }),
                        ))
                        .child(section("Assistant", cx).child(card(cx).child(field(
                            "AI integration",
                            toggle("ai-toggle", s.ai_integration_enabled, |b, on| b.set_ai_integration(on), cx),
                            cx,
                        ))))
                        .child(section("Appearance", cx).child(
                            card(cx)
                                .child(field(
                                    "Appearance",
                                    segmented(
                                        "appearance",
                                        Appearance::ALL.iter().map(|a| (a.label().to_string(), *a)).collect(),
                                        s.appearance,
                                        |b, v, window, cx| {
                                            b.settings.appearance = v;
                                            b.apply_theme(Some(window), cx);
                                        },
                                        cx,
                                    ),
                                    cx,
                                ))
                                .child(field(
                                    "Sidebar side",
                                    segmented(
                                        "sidebar-side",
                                        vec![
                                            ("Left".into(), SidebarPosition::Left),
                                            ("Right".into(), SidebarPosition::Right),
                                        ],
                                        s.sidebar_position,
                                        |b, v, _, _| b.settings.sidebar_position = v,
                                        cx,
                                    ),
                                    cx,
                                ))
                                .child(field(
                                    "Show sidebar on launch",
                                    toggle("sidebar-launch", s.show_sidebar_on_launch, |b, on| b.settings.show_sidebar_on_launch = on, cx),
                                    cx,
                                ))
                                .child(
                                    div()
                                        .text_size(metrics::TEXT_SMALL)
                                        .text_color(t.muted_foreground)
                                        .child(format!("Theme for “{}”", self.ws.active().name)),
                                )
                                .child(v_flex().gap_1().children(preset_rows))
                                .when(has_theme, |c| {
                                    c.child(field(
                                        "Wash",
                                        segmented(
                                            "wash",
                                            opacity_steps.clone(),
                                            current_opacity,
                                            |b, v, window, cx| {
                                                let mut theme = b.ws.active().theme.clone();
                                                theme.opacity = v;
                                                b.set_space_theme(theme, window, cx);
                                            },
                                            cx,
                                        ),
                                        cx,
                                    ))
                                    .child(field(
                                        "Grain",
                                        segmented(
                                            "grain",
                                            grain_steps.clone(),
                                            current_grain,
                                            |b, v, window, cx| {
                                                let mut theme = b.ws.active().theme.clone();
                                                theme.texture = v;
                                                b.set_space_theme(theme, window, cx);
                                            },
                                            cx,
                                        ),
                                        cx,
                                    ))
                                })
                                .child(field("Solid color", h_flex().gap_2().children(swatches), cx)),
                        ))
                        .child(section("Tabs", cx).child(
                            card(cx)
                                .child(field(
                                    "Sleep idle tabs",
                                    segmented(
                                        "sleep",
                                        SLEEP_OPTIONS.iter().map(|(v, l)| (l.to_string(), *v)).collect(),
                                        s.auto_sleep_minutes,
                                        |b, v, _, _| b.settings.auto_sleep_minutes = v,
                                        cx,
                                    ),
                                    cx,
                                ))
                                .child(field(
                                    "Archive idle tabs",
                                    segmented(
                                        "archive",
                                        ARCHIVE_OPTIONS.iter().map(|(v, l)| (l.to_string(), *v)).collect(),
                                        s.auto_archive_hours,
                                        |b, v, _, _| b.settings.auto_archive_hours = v,
                                        cx,
                                    ),
                                    cx,
                                )),
                        ))
                        .child(section("Air Traffic Control", cx).child(
                            card(cx)
                                .child(
                                    div()
                                        .text_size(metrics::TEXT_SMALL)
                                        .text_color(t.muted_foreground)
                                        .child("Pages from these sites move into their space automatically."),
                                )
                                .children(routes)
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .child(div().flex_1().child(Input::new(&route_pattern)))
                                        .child(
                                            text_button("route-add", format!("Route to {}", self.ws.active().name), true, cx)
                                                .on_click(cx.listener({
                                                    let input = route_pattern.clone();
                                                    move |this, _, _, cx| {
                                                        let value = input.read(cx).value().to_string();
                                                        this.add_route(&value, cx);
                                                    }
                                                })),
                                        ),
                                ),
                        ))
                        .child(section("Boosts", cx).child(
                            card(cx)
                                .when(boosts.is_empty(), |c| {
                                    c.child(
                                        div()
                                            .text_color(t.muted_foreground)
                                            .child("No boosts yet. Open a site and press Ctrl+Shift+B."),
                                    )
                                })
                                .children(boosts),
                        ))
                        .child(section("About", cx).child(
                            card(cx).child(
                                h_flex()
                                    .gap_3()
                                    .child(icon("trees", px(28.), t.primary))
                                    .child(
                                        v_flex()
                                            .child(div().font_weight(FontWeight::SEMIBOLD).child(format!(
                                                "Mori {}",
                                                env!("CARGO_PKG_VERSION")
                                            )))
                                            .child(
                                                div()
                                                    .text_size(metrics::TEXT_SMALL)
                                                    .text_color(t.muted_foreground)
                                                    .child(format!(
                                                        "GPUI chrome · Chromium {} via CEF",
                                                        mori_engine::chromium_version()
                                                    )),
                                            ),
                                    ),
                            ),
                        )),
                ),
            )
    }
}
