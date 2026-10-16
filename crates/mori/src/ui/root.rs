//! Window layout: the unified chrome surface, sidebar, web card, assistant
//! panel, and every overlay, plus action routing.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use mori_core::settings::SidebarPosition;

use crate::{
    actions::*,
    browser::{Browser, PageOwner, Panel},
    commands::{self, Command},
    theme::{ActiveTokens, metrics},
    ui::{
        components::{h_flex, icon, text_button, v_flex},
        launcher::LauncherMode,
    },
};

impl Browser {
    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Esc priority chain from the original app; if nothing handles it,
        // the key goes on to the page.
        if self.zapping.is_some() {
            self.finish_zap(cx);
        } else if self.cancel_front_modal(window, cx) {
            // A page's dialog or sign-in prompt took the key.
        } else if self.menu.is_some() {
            // A menu can sit over the Peek (opened from the sidebar).
            self.close_menu(window, cx);
        } else if self.boost_editor.is_some() {
            self.boost_editor = None;
        } else if self.space_editor.is_some() {
            self.close_space_editor(window, cx);
        } else if self.peek.is_some() {
            self.close_peek(window, cx);
        } else if self.popup.is_some() {
            self.close_popup_window(window, cx);
        } else if self.launcher.is_some() {
            self.close_launcher(window, cx);
        } else if self.rename.is_some() {
            self.cancel_rename(window, cx);
        } else if self.panel.is_some() {
            self.panel = None;
        } else if self.find_bar.is_some() {
            self.close_find(window, cx);
        } else if self.settings_open {
            self.set_settings_open(false, window, cx);
        } else if self.selected_live().is_some_and(|l| l.error.is_some()) {
            if let Some(id) = self.selected()
                && let Some(l) = self.live.get_mut(&id)
            {
                l.error = None;
            }
        } else if self.selected_live().is_some_and(|l| l.fullscreen) {
            if let Some(v) = self.selected_view() {
                v.exit_fullscreen();
            }
        } else {
            cx.propagate();
            return;
        }
        cx.notify();
    }

    /// The space's gradient wash (and grain), filling its parent. `radius`
    /// rounds the wash to match a rounded parent; GPUI's clipping is
    /// rectangular, so `overflow_hidden` alone leaves square corners.
    fn render_wash(&self, radius: Pixels, cx: &App) -> Option<AnyElement> {
        let t = cx.tokens();
        let wash = t.wash.clone()?;
        let grain = t.grain.clone();
        let opacity = t.gradient.clamped_opacity();
        Some(
            canvas(
                |_, _, _| (),
                move |bounds, _, window, _| {
                    let _ = window.paint_image(bounds, bounds, Corners::all(radius), wash, 0, false);
                    if let Some(grain) = grain {
                        let tile = px(128.);
                        let mut y = bounds.origin.y;
                        while y < bounds.bottom() {
                            let mut x = bounds.origin.x;
                            while x < bounds.right() {
                                let b = Bounds::new(point(x, y), size(tile, tile));
                                let _ = window.paint_image(bounds, b, Corners::default(), grain.clone(), 0, false);
                                x += tile;
                            }
                            y += tile;
                        }
                    }
                },
            )
            .absolute()
            .inset_0()
            .opacity(opacity)
            .into_any_element(),
        )
    }

    fn render_card(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.tokens().clone();
        let selected = self.selected();
        let url = selected
            .and_then(|id| self.ws.tab(id))
            .map(|r| r.url.clone())
            .unwrap_or_default();

        let content: AnyElement = if self.settings_open {
            self.render_settings(window, cx).into_any_element()
        } else if let Some(live) = self.selected_live() {
            live.surface.clone().into_any_element()
        } else if Browser::shows_new_tab_page(&url) {
            self.render_new_tab_page(cx).into_any_element()
        } else {
            div().size_full().into_any_element()
        };

        let live = self.selected_live();
        let error = live.and_then(|l| l.error.clone()).filter(|_| !self.settings_open);
        // Link targets hide while a menu, the launcher, a Peek or a popup
        // covers the page (the pointer left it without a mouse-leave).
        let status = live.and_then(|l| l.status.clone()).filter(|_| {
            !self.settings_open
                && self.menu.is_none()
                && self.launcher.is_none()
                && self.peek.is_none()
                && self.popup.is_none()
        });
        let fullscreen = live.is_some_and(|l| l.fullscreen);
        // In a split, the pane with the user's attention gets an accent ring.
        let active_pane = self.split.is_some() && !self.split_focused && !self.settings_open;

        let mut card = div()
            .id("web-card")
            .relative()
            .size_full()
            .bg(t.card)
            .when(!fullscreen, |s| s.rounded(metrics::CARD_RADIUS).shadow(t.card_shadow()))
            .when(active_pane, |s| s.shadow(active_pane_ring(t.primary)))
            .child(content);

        if let Some(status) = status {
            // Zen's status panel: flush in the card's bottom-left corner,
            // only the inner corner rounded.
            card = card.child(
                div()
                    .absolute()
                    .bottom_0()
                    .left_0()
                    .max_w(relative(0.5))
                    .px(px(8.))
                    .py(px(4.))
                    .rounded_tr(metrics::CARD_RADIUS)
                    .when(!fullscreen, |s| s.rounded_bl(metrics::CARD_RADIUS))
                    .bg(if t.dark { rgb(0x2B2B2E).into() } else { t.popover })
                    .when(!t.dark, |s| s.border_t_1().border_r_1().border_color(t.popover_border()))
                    .text_size(metrics::TEXT_BASE)
                    .text_color(t.foreground.opacity(0.9))
                    .truncate()
                    .child(status),
            );
        }
        if let Some(bar) = &self.find_bar
            && !self.settings_open
        {
            card = card.child(bar.clone());
        }
        if let Some(err) = error {
            card = card.child(self.render_error(err, cx));
        }
        // Over a Peek or popup the toasts move above them (see render).
        if !self.page_overlay_open()
            && let Some(toasts) = self.render_toasts(cx)
        {
            card = card.child(toasts);
        }
        if let Some(id) = selected.filter(|_| !self.settings_open) {
            card = card.children(self.render_page_prompts(PageOwner::Tab(id), cx));
        }
        card.drag_over::<crate::ui::sidebar::DraggedTab>(move |s, _, _, _| {
            s.border_2().border_color(t.primary.opacity(0.7))
        })
        .on_drop(cx.listener(|this, d: &crate::ui::sidebar::DraggedTab, window, cx| {
            this.open_split(d.id, window, cx)
        }))
        .into_any_element()
    }

    /// The second page of a split view. Hovering it shows a small pill at
    /// the top centre (clear of the page's scrollbar) to end the split.
    fn render_split_card(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let id = self.split?;
        let live = self.live.get(&id)?;
        let t = cx.tokens().clone();
        let surface = live.surface.clone();
        let prompts = self.render_page_prompts(PageOwner::Tab(id), cx);
        let error = live.error.clone();
        let focused = self.split_focused;
        Some(
            div()
                .id("split-card")
                .group("split-card")
                .relative()
                .size_full()
                .bg(t.card)
                .rounded(metrics::CARD_RADIUS)
                .shadow(if focused { active_pane_ring(t.primary) } else { t.card_shadow() })
                .child(surface)
                .children(prompts)
                .when_some(error, |s, err| s.child(self.render_split_error(id, err, cx)))
                .child(
                    h_flex()
                        .absolute()
                        .top(px(6.))
                        .left_0()
                        .right_0()
                        .justify_center()
                        .child(
                            h_flex()
                                .id("close-split")
                                .h(px(22.))
                                .px_2()
                                .gap_1()
                                .rounded_full()
                                .bg(t.popover.opacity(0.95))
                                .border_1()
                                .border_color(t.popover_border())
                                .shadow(t.popover_shadow())
                                .text_size(metrics::TEXT_SMALL)
                                .text_color(t.muted_foreground)
                                .cursor_pointer()
                                .invisible()
                                .group_hover("split-card", |s| s.visible())
                                .hover(|s| s.text_color(t.foreground))
                                .child(icon("columns-2", px(11.), t.muted_foreground))
                                .child("Close Split")
                                .on_click(cx.listener(|this, _, _, cx| this.close_split(cx))),
                        ),
                )
                .into_any_element(),
        )
    }

    /// The split pane's load error, with a Reload button for that pane.
    fn render_split_error(
        &self,
        id: mori_core::workspace::TabId,
        err: crate::browser::PageError,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = cx.tokens().clone();
        v_flex()
            .absolute()
            .inset_0()
            .items_center()
            .justify_center()
            .gap_3()
            .bg(t.card)
            .rounded(metrics::RADIUS)
            .text_center()
            .child(icon("cloud-off", px(28.), t.muted_foreground))
            .child(div().font_weight(FontWeight::SEMIBOLD).child("This page couldn't load"))
            .child(
                div()
                    .text_size(metrics::TEXT_LABEL)
                    .text_color(t.muted_foreground)
                    .child(format!("{} ({})", err.text, err.code)),
            )
            .child(text_button("split-err-reload", "Reload", true, cx).on_click(cx.listener(
                move |this, _, _, cx| {
                    if let Some(l) = this.live.get_mut(&id) {
                        l.error = None;
                        l.view.reload(false);
                    }
                    cx.notify();
                },
            )))
    }

    /// The 8px gap between split panes, draggable to resize them.
    fn render_split_divider(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.tokens().clone();
        let dragging = self.split_drag.is_some();
        div()
            .id("split-divider")
            .w(metrics::CARD_INSET)
            .h_full()
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .cursor_col_resize()
            .group("split-divider")
            .child(
                div()
                    .w(px(3.))
                    .h(px(36.))
                    .rounded_full()
                    .invisible()
                    .when(dragging, |s| s.visible())
                    .group_hover("split-divider", |s| s.visible())
                    .bg(t.sidebar_foreground.opacity(0.28)),
            )
            .on_click(cx.listener(|this, ev: &ClickEvent, _, cx| {
                if ev.click_count() == 2 {
                    this.finish_split_divider(true, cx);
                }
            }))
            .on_drag(SplitResize, |_, _, _, cx| cx.new(|_| EmptyView))
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.finish_split_divider(false, cx)),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.finish_split_divider(false, cx)),
            )
    }

    fn render_error(&self, err: crate::browser::PageError, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.tokens().clone();
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(t.card)
            .rounded(metrics::CARD_RADIUS)
            .child(
                v_flex()
                    .max_w(px(360.))
                    .gap_3()
                    .items_center()
                    .text_center()
                    .child(icon("cloud-off", px(32.), t.muted_foreground))
                    .child(
                        div()
                            .text_size(metrics::TEXT_TITLE)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("This page couldn't load"),
                    )
                    .child(
                        div()
                            .font_family("monospace")
                            .text_size(metrics::TEXT_LABEL)
                            .text_color(t.muted_foreground)
                            .child(err.url.clone()),
                    )
                    .child(
                        div()
                            .text_size(metrics::TEXT_LABEL)
                            .text_color(t.muted_foreground)
                            .child(format!("{} ({})", err.text, err.code)),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(text_button("err-dismiss", "Dismiss", false, cx).on_click(
                                cx.listener(|this, _, _, cx| {
                                    if let Some(id) = this.selected()
                                        && let Some(l) = this.live.get_mut(&id)
                                    {
                                        l.error = None;
                                    }
                                    cx.notify();
                                }),
                            ))
                            .child(text_button("err-reload", "Reload", true, cx).on_click(
                                cx.listener(|this, _, _, cx| {
                                    if let Some(id) = this.selected()
                                        && let Some(l) = this.live.get_mut(&id)
                                    {
                                        l.error = None;
                                        l.view.reload(false);
                                    }
                                    cx.notify();
                                }),
                            )),
                    ),
            )
    }

    fn render_new_tab_page(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.tokens().clone();
        v_flex()
            .id("new-tab-page")
            .size_full()
            .items_center()
            .justify_center()
            .gap_4()
            .child(
                svg()
                    .path("mori/logo.svg")
                    .size(px(56.))
                    .text_color(t.foreground.opacity(0.18)),
            )
            .child(
                h_flex()
                    .id("ntp-search")
                    .w(px(420.))
                    .h(px(44.))
                    .px_4()
                    .gap_3()
                    .rounded(metrics::RADIUS_POPOVER)
                    .bg(t.foreground.opacity(0.05))
                    .border_1()
                    .border_color(t.border.opacity(0.6))
                    .text_color(t.muted_foreground)
                    .cursor_text()
                    .child(icon("search", px(16.), t.muted_foreground))
                    .child(div().flex_1().child("Search or enter address"))
                    .child(crate::ui::components::shortcut_caps("secondary-l", cx))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_launcher(LauncherMode::Address, window, cx)
                    })),
            )
    }
}

/// The accent ring around the active pane of a split view (a shadow, so it
/// doesn't change the page's size).
fn active_pane_ring(color: Hsla) -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: color.opacity(0.55),
        offset: point(px(0.), px(0.)),
        blur_radius: px(0.),
        spread_radius: px(1.5),
        inset: false,
    }]
}

/// Zen's page-load indicator: a small pill in the gutter above the web
/// card, pulsing while the page loads.
fn loading_pill(cx: &App) -> impl IntoElement {
    let t = cx.tokens();
    let color = if t.dark { white().opacity(0.22) } else { black().opacity(0.2) };
    div()
        .absolute()
        .top(px(1.5))
        .left_0()
        .right_0()
        .flex()
        .justify_center()
        .child(
            div()
                .w(px(64.))
                .h(px(5.))
                .rounded_full()
                .bg(color)
                .with_animation(
                    "loading-pulse",
                    Animation::new(std::time::Duration::from_millis(1200))
                        .repeat()
                        .with_easing(ease_in_out),
                    |el, delta| el.opacity(0.35 + 0.65 * (delta * std::f32::consts::PI).sin()),
                ),
        )
}

impl Render for Browser {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_view_visibility(cx);
        let t = cx.tokens().clone();
        let fullscreen = self.selected_live().is_some_and(|l| l.fullscreen);
        let show_sidebar = self.settings.sidebar_visible && !fullscreen;
        let left = self.settings.sidebar_position == SidebarPosition::Left;
        let inset = if fullscreen { px(0.) } else { metrics::CARD_INSET };
        let loading = !self.settings_open
            && !fullscreen
            && (self.selected_live().is_some_and(|l| l.loading)
                || self.split.and_then(|id| self.live.get(&id)).is_some_and(|l| l.loading));

        let sidebar = show_sidebar.then(|| self.render_sidebar(false, window, cx));
        let resize_handle = show_sidebar.then(|| self.render_resize_handle(left, cx));
        let card = self.render_card(window, cx);
        let split_card = (!self.settings_open && !fullscreen)
            .then(|| self.render_split_card(cx))
            .flatten();
        let assistant = self
            .assistant
            .clone()
            .filter(|_| self.assistant_visible && !fullscreen);

        let ratio = self.split_ratio();
        let divider = split_card.is_some().then(|| self.render_split_divider(cx));
        let web_column = div()
            .id("web-column")
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .pt(inset)
            .pb(inset)
            .pl(if show_sidebar && left { px(0.) } else { inset })
            .pr(if show_sidebar && !left { px(0.) } else { inset })
            .flex()
            .map(|s| match split_card {
                Some(split_card) => s
                    .child(div().flex_basis(relative(ratio)).flex_grow(1.).min_w_0().h_full().child(card))
                    .children(divider)
                    .child(div().flex_basis(relative(1. - ratio)).flex_grow(1.).min_w_0().h_full().child(split_card)),
                None => s.child(div().flex_1().min_w_0().h_full().child(card)),
            })
            .children(self.render_devtools_card(cx))
            .when(loading, |s| s.child(loading_pill(cx)))
            .on_drag_move(cx.listener(|this, ev: &DragMoveEvent<SplitResize>, _, cx| {
                let bounds = ev.bounds;
                let width = f32::from(bounds.size.width).max(1.);
                let ratio = f32::from(ev.event.position.x - bounds.origin.x) / width;
                this.drag_split_divider(ratio, cx);
            }));

        let mut row = h_flex().size_full().items_stretch();
        if left {
            row = row
                .children(sidebar)
                .children(resize_handle)
                .child(web_column)
                .children(assistant.map(|a| a.into_any_element()));
        } else {
            row = row
                .children(assistant.map(|a| a.into_any_element()))
                .child(web_column)
                .children(resize_handle)
                .children(sidebar);
        }

        // Hidden sidebar: touching the window edge reveals it as a card
        // (see `track_sidebar_peek`).
        let floating = (self.sidebar_peek && !show_sidebar && !fullscreen).then(|| {
            let sidebar = self.render_sidebar(true, window, cx);
            // Opaque, with the space's wash painted inside it, so the page
            // never reads through the tab list.
            div()
                .id("sidebar-peek")
                .absolute()
                .top(px(4.))
                .bottom(px(4.))
                .when(left, |s| s.left(px(4.)))
                .when(!left, |s| s.right(px(4.)))
                .rounded(metrics::RADIUS)
                .overflow_hidden()
                .bg(t.sidebar)
                .border_1()
                .border_color(t.popover_border())
                .shadow(vec![BoxShadow {
                    color: black().opacity(t.shadow_alpha(0.18, 0.45)),
                    offset: point(px(0.), px(8.)),
                    blur_radius: px(28.),
                    spread_radius: px(0.),
                    inset: false,
                }])
                .occlude()
                .children(self.render_wash(metrics::RADIUS, cx))
                .child(sidebar)
        });

        let launcher = self.launcher.clone();
        let menu = self.render_menu(window, cx);
        let panel = self.render_panel(window, cx);
        let peek = self.render_peek(window, cx);
        let popup = self.render_popup_window(cx);
        // Toasts raised while a Peek or popup is open stay visible and
        // clickable above it, over the same page area.
        let overlay_toasts = if self.page_overlay_open() {
            let area = self.peek_area(window, cx);
            self.render_toasts(cx).map(|toasts| {
                div()
                    .absolute()
                    .left(area.left())
                    .top(area.top())
                    .w(area.size.width)
                    .h(area.size.height)
                    .child(toasts)
                    .into_any_element()
            })
        } else {
            None
        };
        let boost = self.boost_editor.clone();
        let space_editor = self.space_editor.clone();

        div()
            .id("mori-root")
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .capture_any_mouse_down(cx.listener(|this, ev: &MouseDownEvent, _, _| {
                if ev.button == MouseButton::Left && ev.modifiers.alt {
                    this.alt_click_at = Some(std::time::Instant::now());
                }
            }))
            .relative()
            .size_full()
            .bg(t.sidebar)
            .text_color(t.foreground)
            .font_family(crate::theme::ui_font().family.clone())
            .text_size(metrics::TEXT_BASE)
            .on_action(cx.listener(|this, _: &NewTab, window, cx| {
                this.open_launcher(LauncherMode::NewTab, window, cx)
            }))
            .on_action(cx.listener(|this, _: &OpenLocation, window, cx| {
                this.open_launcher(LauncherMode::Address, window, cx)
            }))
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| {
                if let Some(id) = this.active_tab() {
                    this.close_tab(id, false, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ReopenClosedTab, window, cx| this.reopen_closed_tab(window, cx)))
            .on_action(cx.listener(|this, _: &NextTab, window, cx| this.cycle_tab(1, window, cx)))
            .on_action(cx.listener(|this, _: &PrevTab, window, cx| this.cycle_tab(-1, window, cx)))
            .on_action(cx.listener(|this, _: &Reload, _, _| {
                if let Some(v) = this.selected_view() {
                    v.reload(false);
                }
            }))
            .on_action(cx.listener(|this, _: &HardReload, _, _| {
                if let Some(v) = this.selected_view() {
                    v.reload(true);
                }
            }))
            .on_action(cx.listener(|this, _: &Stop, _, _| {
                if let Some(v) = this.selected_view() {
                    v.stop();
                }
            }))
            .on_action(cx.listener(|this, _: &Back, _, _| {
                if let Some(v) = this.selected_view() {
                    v.go_back();
                }
            }))
            .on_action(cx.listener(|this, _: &Forward, _, _| {
                if let Some(v) = this.selected_view() {
                    v.go_forward();
                }
            }))
            .on_action(cx.listener(|this, _: &GoHome, window, cx| this.go_home(window, cx)))
            .on_action(cx.listener(|this, _: &Find, window, cx| this.open_find(window, cx)))
            .on_action(cx.listener(|this, _: &FindNext, window, cx| this.find_step(true, window, cx)))
            .on_action(cx.listener(|this, _: &FindPrev, window, cx| this.find_step(false, window, cx)))
            .on_action(cx.listener(|this, _: &ZoomIn, _, cx| this.zoom(0.5, cx)))
            .on_action(cx.listener(|this, _: &ZoomOut, _, cx| this.zoom(-0.5, cx)))
            .on_action(cx.listener(|this, _: &ZoomReset, _, cx| this.zoom(0.0, cx)))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| this.toggle_sidebar(cx)))
            .on_action(cx.listener(|this, _: &ToggleAssistant, window, cx| this.toggle_assistant(window, cx)))
            .on_action(cx.listener(|this, _: &OpenSettings, window, cx| this.toggle_settings(window, cx)))
            .on_action(cx.listener(|this, _: &CopyUrl, _, cx| this.copy_url(cx)))
            .on_action(cx.listener(|this, _: &DuplicateTab, window, cx| {
                if let Some(id) = this.selected() {
                    this.duplicate_tab(id, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &TogglePin, _, cx| {
                if let Some(id) = this.selected() {
                    this.toggle_pin(id, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ToggleBookmark, _, cx| this.toggle_bookmark(cx)))
            .on_action(cx.listener(|this, _: &Print, window, cx| commands::run(this, Command::Print, window, cx)))
            .on_action(cx.listener(|this, _: &ToggleDevTools, window, cx| this.toggle_devtools(window, cx)))
            .on_action(cx.listener(|this, _: &PeekClipboard, window, cx| commands::peek_clipboard(this, window, cx)))
            .on_action(cx.listener(|this, _: &BoostSite, window, cx| this.open_boost_editor(window, cx)))
            .on_action(cx.listener(|this, _: &ZapElement, _, cx| this.start_zap(cx)))
            .on_action(cx.listener(|this, _: &SleepBackgroundTabs, _, cx| this.sleep_background_tabs(cx)))
            .on_action(cx.listener(|this, _: &ToggleReader, _, cx| this.toggle_reader(cx)))
            .on_action(cx.listener(|this, _: &CaptureVisibleTab, _, cx| {
                crate::screenshot::capture_visible_tab(this, cx)
            }))
            .on_action(cx.listener(|_, _: &ToggleFullscreen, window, _| window.toggle_fullscreen()))
            .on_action(cx.listener(|this, _: &ShowLibrary, _, cx| {
                this.panel = (this.panel != Some(Panel::Library)).then_some(Panel::Library);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ShowDownloads, _, cx| {
                this.panel = (this.panel != Some(Panel::Downloads)).then_some(Panel::Downloads);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &NewSpace, window, cx| this.create_space(window, cx)))
            .on_action(cx.listener(|this, _: &NextSpace, window, cx| this.step_space(1, window, cx)))
            .on_action(cx.listener(|this, _: &PrevSpace, window, cx| this.step_space(-1, window, cx)))
            .on_action(cx.listener(|this, _: &CloseSplit, _, cx| this.close_split(cx)))
            .on_action(cx.listener(|this, _: &Dismiss, window, cx| this.dismiss(window, cx)))
            .on_action(cx.listener(|this, _: &Quit, _, cx| this.quit(cx)))
            .on_action(cx.listener(|this, _: &SelectTab1, w, cx| this.select_ordinal(1, w, cx)))
            .on_action(cx.listener(|this, _: &SelectTab2, w, cx| this.select_ordinal(2, w, cx)))
            .on_action(cx.listener(|this, _: &SelectTab3, w, cx| this.select_ordinal(3, w, cx)))
            .on_action(cx.listener(|this, _: &SelectTab4, w, cx| this.select_ordinal(4, w, cx)))
            .on_action(cx.listener(|this, _: &SelectTab5, w, cx| this.select_ordinal(5, w, cx)))
            .on_action(cx.listener(|this, _: &SelectTab6, w, cx| this.select_ordinal(6, w, cx)))
            .on_action(cx.listener(|this, _: &SelectTab7, w, cx| this.select_ordinal(7, w, cx)))
            .on_action(cx.listener(|this, _: &SelectTab8, w, cx| this.select_ordinal(8, w, cx)))
            .on_action(cx.listener(|this, _: &SelectLastTab, w, cx| this.select_ordinal(9, w, cx)))
            .on_action(cx.listener(|this, _: &SelectSpace1, w, cx| select_space(this, 1, w, cx)))
            .on_action(cx.listener(|this, _: &SelectSpace2, w, cx| select_space(this, 2, w, cx)))
            .on_action(cx.listener(|this, _: &SelectSpace3, w, cx| select_space(this, 3, w, cx)))
            .on_action(cx.listener(|this, _: &SelectSpace4, w, cx| select_space(this, 4, w, cx)))
            .on_action(cx.listener(|this, _: &SelectSpace5, w, cx| select_space(this, 5, w, cx)))
            .on_action(cx.listener(|this, _: &SelectSpace6, w, cx| select_space(this, 6, w, cx)))
            .on_action(cx.listener(|this, _: &SelectSpace7, w, cx| select_space(this, 7, w, cx)))
            .on_action(cx.listener(|this, _: &SelectSpace8, w, cx| select_space(this, 8, w, cx)))
            .on_action(cx.listener(|this, _: &SelectSpace9, w, cx| select_space(this, 9, w, cx)))
            .children(self.render_wash(px(0.), cx))
            .child(row)
            .child(self.render_pointer_hooks(cx))
            .children(floating)
            .children(panel)
            .children(peek)
            .children(popup)
            .children(overlay_toasts)
            .children(boost.map(|b| b.into_any_element()))
            .children(space_editor.map(|e| e.into_any_element()))
            .children(launcher.map(|l| l.into_any_element()))
            .children(menu)
    }
}

fn select_space(b: &mut Browser, n: usize, window: &mut Window, cx: &mut Context<Browser>) {
    if let Some(id) = b.ws.space_by_ordinal(n) {
        b.switch_space(id, window, cx);
    }
}

impl Browser {
    fn render_resize_handle(&mut self, left: bool, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.tokens().clone();
        let dragging = self.sidebar_drag.is_some();
        div()
            .id("sidebar-resize")
            .w(px(8.))
            .h_full()
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .cursor_col_resize()
            .group("resize")
            .child(
                div()
                    .w(px(3.))
                    .h(px(36.))
                    .rounded_full()
                    .invisible()
                    .when(dragging, |s| s.visible())
                    .group_hover("resize", |s| s.visible())
                    .bg(t.sidebar_foreground.opacity(0.28)),
            )
            .on_click(cx.listener(|this, ev: &ClickEvent, _, cx| {
                if ev.click_count() == 2 {
                    this.settings.sidebar_width = metrics::SIDEBAR_DEFAULT;
                    this.schedule_save(cx);
                    cx.notify();
                }
            }))
            .on_drag(SidebarResize, |_, _, _, cx| cx.new(|_| EmptyView))
            .on_drag_move(cx.listener(move |this, ev: &DragMoveEvent<SidebarResize>, window, cx| {
                let x = f32::from(ev.event.position.x);
                let width = if left {
                    x
                } else {
                    f32::from(window.viewport_size().width) - x
                };
                this.sidebar_drag = Some(width.clamp(metrics::SIDEBAR_MIN, metrics::SIDEBAR_MAX));
                cx.notify();
            }))
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.finish_sidebar_drag(cx)),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.finish_sidebar_drag(cx)),
            )
    }

    fn finish_sidebar_drag(&mut self, cx: &mut Context<Self>) {
        if let Some(w) = self.sidebar_drag.take() {
            self.settings.sidebar_width = w;
            self.schedule_save(cx);
            cx.notify();
        }
    }
}

#[derive(Clone)]
struct SidebarResize;

#[derive(Clone)]
struct SplitResize;

struct EmptyView;

impl Render for EmptyView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}
