//! The first-run welcome (Arc/Zen style): a few full-window steps to pick
//! the look, bring bookmarks, history and tabs over from another browser,
//! and learn the handful of shortcuts that matter. Shown once per profile;
//! finishing or skipping it marks the profile onboarded.

use std::time::Duration;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use mori_core::{
    settings::{Appearance, SidebarPosition},
    theme::{GradientTheme, presets},
};

use crate::{
    browser::Browser,
    theme::{ActiveTokens, metrics},
    ui::components::{h_flex, icon, shortcut_caps, text_button, v_flex},
};

const CARD_WIDTH: Pixels = px(580.);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnboardingStep {
    Welcome,
    Appearance,
    Import,
    Done,
}

impl OnboardingStep {
    const ALL: [OnboardingStep; 4] = [
        OnboardingStep::Welcome,
        OnboardingStep::Appearance,
        OnboardingStep::Import,
        OnboardingStep::Done,
    ];

    fn index(self) -> usize {
        Self::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }

    fn next(self) -> Option<Self> {
        Self::ALL.get(self.index() + 1).copied()
    }

    fn previous(self) -> Option<Self> {
        self.index().checked_sub(1).map(|i| Self::ALL[i])
    }
}

impl Browser {
    /// Show the welcome on a profile that hasn't seen it, unless Mori was
    /// started to open links (they come first; the welcome waits for the
    /// next launch).
    pub fn maybe_start_onboarding(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.settings.needs_onboarding() || std::env::var_os("MORI_START_URL").is_some() {
            return;
        }
        // Runs after the links from the command line were opened.
        cx.defer_in(window, |this, window, cx| {
            let opened_links = this.ws.tabs.iter().any(|t| !Browser::is_internal(&t.url));
            if opened_links || this.onboarding.is_some() {
                return;
            }
            if this.launcher.is_some() {
                this.close_launcher(window, cx);
            }
            this.onboarding = Some(OnboardingStep::Welcome);
            this.detect_import_sources(cx);
            window.focus(&this.focus_handle, cx);
            cx.notify();
        });
    }

    /// Finish (or skip) the welcome.
    pub fn finish_onboarding(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.onboarding.take().is_none() {
            return;
        }
        self.settings.onboarded = true;
        self.schedule_save(cx);
        // The blank start tab's command bar, which the welcome held back.
        let blank = self.selected().and_then(|id| self.ws.tab(id)).is_some_and(|t| Browser::is_internal(&t.url));
        if blank && self.launcher.is_none() {
            self.open_launcher(crate::ui::launcher::LauncherMode::NewTab, window, cx);
        } else {
            self.focus_selected_page(window, cx);
        }
        cx.notify();
    }

    fn set_onboarding_step(&mut self, step: OnboardingStep, cx: &mut Context<Self>) {
        self.onboarding = Some(step);
        cx.notify();
    }

    pub fn render_onboarding(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let step = self.onboarding?;
        let t = cx.tokens().clone();
        let viewport = window.viewport_size();
        let width = CARD_WIDTH.min(viewport.width - px(32.));
        let body: AnyElement = match step {
            OnboardingStep::Welcome => self.render_welcome(cx),
            OnboardingStep::Appearance => self.render_appearance_step(cx),
            OnboardingStep::Import => self.render_import_step(cx),
            OnboardingStep::Done => self.render_done_step(cx),
        };
        let (panel_bg, border): (Hsla, Hsla) = if t.dark {
            (rgb(0x181818).into(), rgb(0x3a3a3a).into())
        } else {
            (white(), t.popover_border())
        };
        let importing = self.import.is_running();
        let primary_label = match step {
            OnboardingStep::Welcome => "Get Started",
            OnboardingStep::Done => "Start Browsing",
            OnboardingStep::Import if self.import.finished_with_changes() => "Continue",
            OnboardingStep::Import => "Skip",
            OnboardingStep::Appearance => "Continue",
        };
        let dots = h_flex().gap(px(6.)).children(OnboardingStep::ALL.iter().map(|s| {
            let current = *s == step;
            div()
                .h(px(6.))
                .w(if current { px(18.) } else { px(6.) })
                .rounded_full()
                .bg(if current { t.primary } else { t.foreground.opacity(0.18) })
        }));
        let footer = h_flex()
            .gap(px(10.))
            .child(dots)
            .child(div().flex_1())
            .when(step == OnboardingStep::Welcome, |f| {
                f.child(
                    div()
                        .id("onboarding-skip")
                        .px(px(10.))
                        .h(px(30.))
                        .flex()
                        .items_center()
                        .rounded(metrics::RADIUS_MD)
                        .cursor_pointer()
                        .text_color(t.muted_foreground)
                        .hover(|s| s.bg(t.ghost_hover()))
                        .child("Skip setup")
                        .on_click(cx.listener(|this, _, window, cx| this.finish_onboarding(window, cx))),
                )
            })
            .when_some(step.previous().filter(|_| step != OnboardingStep::Done), |f, previous| {
                f.child(text_button("onboarding-back", "Back", false, cx).on_click(cx.listener(
                    move |this, _, _, cx| {
                        if !this.import.is_running() {
                            this.set_onboarding_step(previous, cx);
                        }
                    },
                )))
            })
            .child(
                text_button("onboarding-next", primary_label, true, cx)
                    .when(importing, |b| b.opacity(0.5).cursor_default())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if this.import.is_running() {
                            return;
                        }
                        match step.next() {
                            Some(next) => this.set_onboarding_step(next, cx),
                            None => this.finish_onboarding(window, cx),
                        }
                    })),
            );

        Some(
            div()
                .id("onboarding")
                .absolute()
                .inset_0()
                .occlude()
                .bg(t.sidebar)
                .children(self.render_wash(px(0.), cx))
                .child(div().absolute().inset_0().bg(black().opacity(if t.dark { 0.25 } else { 0.06 })))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    v_flex()
                        .id(("onboarding-card", step.index()))
                        .relative()
                        .w(width)
                        .max_h(viewport.height - px(32.))
                        .p(px(28.))
                        .gap(px(22.))
                        .bg(panel_bg)
                        .rounded(px(16.))
                        .border_1()
                        .border_color(border)
                        .shadow(t.modal_shadow())
                        .text_color(t.foreground)
                        .child(div().id("onboarding-body").flex_1().min_h_0().overflow_y_scroll().child(body))
                        .child(footer)
                        .with_animation(
                            ("onboarding-step", step.index()),
                            Animation::new(Duration::from_millis(220)).with_easing(ease_out_quint()),
                            |el, delta| el.opacity(delta).mt(px(10.) * (1. - delta)),
                        ),
                )
                .into_any_element(),
        )
    }

    fn render_welcome(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.tokens().clone();
        v_flex()
            .items_center()
            .text_center()
            .gap(px(14.))
            .pt(px(18.))
            .pb(px(8.))
            .child(svg().path("mori/logo.svg").size(px(64.)).text_color(t.primary))
            .child(div().text_size(px(26.)).font_weight(FontWeight::BOLD).child("Welcome to Mori"))
            .child(
                div()
                    .max_w(px(420.))
                    .text_size(px(14.))
                    .text_color(t.muted_foreground)
                    .child(
                        "Spaces for each part of your life, pinned tabs that stay put, and a sidebar that \
                         keeps out of your way. Let's set things up — it takes a minute.",
                    ),
            )
            .into_any_element()
    }

    fn render_appearance_step(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.tokens().clone();
        let appearance = self.settings.appearance;
        let tiles = Appearance::ALL.iter().enumerate().map(|(i, a)| {
            let a = *a;
            let active = a == appearance;
            let glyph = match a {
                Appearance::System => "monitor",
                Appearance::Light => "sun",
                Appearance::Dark => "moon",
            };
            v_flex()
                .id(("onboarding-appearance", i))
                .flex_1()
                .items_center()
                .gap(px(8.))
                .py(px(14.))
                .rounded(metrics::RADIUS)
                .border_1()
                .border_color(if active { t.primary.opacity(0.9) } else { t.border.opacity(0.6) })
                .when(active, |s| s.bg(t.primary.opacity(0.10)))
                .when(!active, |s| s.hover(|s| s.bg(t.ghost_hover())))
                .cursor_pointer()
                .child(icon(glyph, px(20.), if active { t.primary } else { t.muted_foreground }))
                .child(div().font_weight(FontWeight::MEDIUM).child(a.label()))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.settings.appearance = a;
                    this.apply_theme(Some(window), cx);
                    this.schedule_save(cx);
                    cx.notify();
                }))
        });

        let current = self.ws.active().theme.clone();
        let swatches = std::iter::once(None).chain(presets().into_iter().map(Some)).enumerate().map(|(i, preset)| {
            let (name, colors, theme) = match &preset {
                None => ("None".to_string(), Vec::new(), GradientTheme::none()),
                Some(p) => (p.name.to_string(), p.colors.to_vec(), p.theme()),
            };
            let active = match &preset {
                None => current.is_none(),
                Some(p) => current.preset_id.as_deref() == Some(p.id),
            };
            // A disc shading from the preset's first colour to its last
            // (backgrounds follow rounded corners; children wouldn't).
            let fill: Background = match (colors.first(), colors.last()) {
                (Some(a), Some(b)) => linear_gradient(
                    135.,
                    linear_color_stop(rgb(*a), 0.),
                    linear_color_stop(rgb(*b), 1.),
                ),
                _ => t.sidebar.into(),
            };
            div()
                .id(("onboarding-theme", i))
                .size(px(34.))
                .flex_none()
                .p(px(3.))
                .rounded_full()
                .border_2()
                .border_color(if active { t.foreground } else { transparent_black() })
                .cursor_pointer()
                .child(
                    div()
                        .size_full()
                        .rounded_full()
                        .bg(fill)
                        .border_1()
                        .border_color(t.border.opacity(0.6)),
                )
                .tooltip({
                    let name: SharedString = name.into();
                    move |window, cx| gpui_kit::component::tooltip::Tooltip::new(name.clone()).build(window, cx)
                })
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.ws.active_mut().theme = theme.clone();
                    this.apply_theme(Some(window), cx);
                    this.schedule_save(cx);
                    cx.notify();
                }))
        });

        let side = self.settings.sidebar_position;
        let side_choice = |id: &'static str, label: &'static str, value: SidebarPosition, cx: &mut Context<Self>| {
            let active = side == value;
            div()
                .id(id)
                .px_3()
                .h(px(26.))
                .flex()
                .items_center()
                .rounded(metrics::RADIUS_LG)
                .text_size(metrics::TEXT_LABEL)
                .cursor_pointer()
                .when(active, |s| {
                    s.bg(if t.dark { white().opacity(0.16) } else { white() })
                        .font_weight(FontWeight::MEDIUM)
                        .when(!t.dark, |s| s.shadow_sm())
                })
                .when(!active, |s| s.text_color(t.foreground.opacity(0.75)).hover(|s| s.bg(t.ghost_hover())))
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings.sidebar_position = value;
                    this.schedule_save(cx);
                    cx.notify();
                }))
        };

        v_flex()
            .gap(px(18.))
            .child(heading("Make it yours", "Pick a look. You can change all of this later in Settings.", cx))
            .child(labelled("Appearance", h_flex().gap(px(10.)).children(tiles), cx))
            .child(labelled(
                &format!("Theme for “{}”", self.ws.active().name),
                h_flex().flex_wrap().gap(px(6.)).children(swatches),
                cx,
            ))
            .child(labelled(
                "Sidebar",
                h_flex().child(
                h_flex()
                    .gap_0p5()
                    .p_0p5()
                    .rounded(px(8.))
                    .bg(t.foreground.opacity(0.06))
                    .child(side_choice("onboarding-left", "Left", SidebarPosition::Left, cx))
                    .child(side_choice("onboarding-right", "Right", SidebarPosition::Right, cx)),
                ),
                cx,
            ))
            .into_any_element()
    }

    fn render_import_step(&mut self, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .gap(px(18.))
            .child(heading(
                "Bring your stuff",
                "Import bookmarks, history and open tabs from the browser you used until now. Zen workspaces \
                 and Arc spaces become spaces, with their pinned tabs and folders.",
                cx,
            ))
            .child(self.render_import_panel(cx))
            .into_any_element()
    }

    fn render_done_step(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.tokens().clone();
        let tip = |keys: &'static str, text: &'static str, cx: &App| {
            h_flex()
                .gap(px(12.))
                .min_h(px(34.))
                .child(div().w(px(120.)).flex_none().child(shortcut_caps(keys, cx)))
                .child(div().text_color(t.foreground.opacity(0.85)).child(text))
        };
        v_flex()
            .gap(px(18.))
            .child(heading("You're all set", "A few keys worth knowing:", cx))
            .child(
                v_flex()
                    .gap(px(2.))
                    .child(tip("secondary-t", "Search, open a page or run a command", cx))
                    .child(tip("ctrl-tab", "Jump back to the tab you used before", cx))
                    .child(tip("secondary-s", "Show or hide the sidebar", cx))
                    .child(tip("secondary-alt-c", "Compact mode: just the page", cx))
                    .child(tip("secondary-/", "Every keyboard shortcut", cx)),
            )
            .into_any_element()
    }
}

fn heading(title: &str, subtitle: &str, cx: &App) -> Div {
    let t = cx.tokens();
    v_flex()
        .gap(px(6.))
        .child(div().text_size(px(20.)).font_weight(FontWeight::BOLD).child(title.to_string()))
        .child(div().text_color(t.muted_foreground).child(subtitle.to_string()))
}

fn labelled(label: &str, control: impl IntoElement, cx: &App) -> Div {
    let t = cx.tokens();
    v_flex()
        .gap(px(8.))
        .child(
            div()
                .text_size(metrics::TEXT_SMALL)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.muted_foreground)
                .child(label.to_uppercase()),
        )
        .child(control)
}
