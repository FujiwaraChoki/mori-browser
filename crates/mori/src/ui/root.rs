//! Window layout: the unified chrome surface, sidebar, web card, assistant
//! panel, and every overlay, plus action routing. Also compact mode (Zen):
//! the sidebar leaves the layout, the page fills the window, and touching
//! the window edge slides the sidebar in over the page.

use std::time::Duration;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use mori_core::settings::{Settings, SidebarPosition};

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

/// How long the docked sidebar takes to slide in or out.
const SIDEBAR_MOTION: Duration = Duration::from_millis(200);
/// The floating sidebar's slide in / out.
const SIDEBAR_PEEK_IN: Duration = Duration::from_millis(180);
const SIDEBAR_PEEK_OUT: Duration = Duration::from_millis(140);
/// The page's margin to the window edges in compact mode.
const COMPACT_INSET: Pixels = px(4.);

/// The page's margin to the window edges and the sidebar.
pub fn card_inset(settings: &Settings) -> Pixels {
    if settings.compact_mode { COMPACT_INSET } else { metrics::CARD_INSET }
}

/// Sidebar slide animations.
#[derive(Default)]
pub struct SidebarFx {
    /// The docked sidebar sliding in (`true`) or out, with its animation id.
    motion: Option<(bool, usize)>,
    /// The floating sidebar playing its slide-out.
    peek_leaving: bool,
    /// Bumped per animation so each starts from its beginning.
    serial: usize,
    _motion_done: Option<Task<()>>,
    _peek_done: Option<Task<()>>,
}

impl SidebarFx {
    pub fn peek_leaving(&self) -> bool {
        self.peek_leaving
    }
}

impl Browser {
    /// Show or hide the docked sidebar and turn compact mode on or off,
    /// sliding the sidebar in or out.
    pub fn set_sidebar_layout(&mut self, visible: bool, compact: bool, cx: &mut Context<Self>) {
        let was_docked = self.settings.docks_sidebar();
        self.settings.sidebar_visible = visible;
        self.settings.compact_mode = compact;
        let docked = self.settings.docks_sidebar();
        self.sidebar_peek = false;
        self.sidebar_fx.peek_leaving = false;
        self.sidebar_hide = None;
        if was_docked != docked {
            self.sidebar_fx.serial += 1;
            self.sidebar_fx.motion = Some((docked, self.sidebar_fx.serial));
            self.sidebar_fx._motion_done = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(SIDEBAR_MOTION).await;
                this.update(cx, |this, cx| {
                    this.sidebar_fx.motion = None;
                    cx.notify();
                })
                .ok();
            }));
        }
        self.schedule_save(cx);
        cx.notify();
    }

    /// Ctrl+Alt+C: compact mode on or off.
    pub fn toggle_compact_mode(&mut self, cx: &mut Context<Self>) {
        let compact = !self.settings.compact_mode;
        // Leaving compact mode brings the sidebar back.
        let visible = self.settings.sidebar_visible || !compact;
        self.set_sidebar_layout(visible, compact, cx);
        if compact {
            let side = match self.settings.sidebar_position {
                SidebarPosition::Left => "left",
                SidebarPosition::Right => "right",
            };
            self.toast(
                format!("Compact mode: touch the {side} edge for the sidebar"),
                crate::browser::ToastStyle::Info,
                None,
                cx,
            );
        }
    }

    /// Slide the floating sidebar in (the pointer touched the window edge).
    pub fn show_sidebar_peek(&mut self, cx: &mut Context<Self>) {
        self.sidebar_peek = true;
        self.sidebar_fx.peek_leaving = false;
        self.sidebar_fx._peek_done = None;
        self.sidebar_fx.serial += 1;
        cx.notify();
    }

    /// Slide the floating sidebar out, then drop it.
    pub fn hide_sidebar_peek(&mut self, cx: &mut Context<Self>) {
        if !self.sidebar_peek || self.sidebar_fx.peek_leaving {
            return;
        }
        self.sidebar_fx.peek_leaving = true;
        self.sidebar_fx.serial += 1;
        self.sidebar_fx._peek_done = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SIDEBAR_PEEK_OUT).await;
            this.update(cx, |this, cx| {
                if this.sidebar_fx.peek_leaving {
                    this.sidebar_fx.peek_leaving = false;
                    this.sidebar_peek = false;
                    cx.notify();
                }
            })
            .ok();
        }));
        cx.notify();
    }

    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Esc priority chain from the original app; if nothing handles it,
        // the key goes on to the page.
        if self.tab_switcher.is_some() {
            self.cancel_tab_switcher(window, cx);
        } else if self.capture.is_some() {
            self.cancel_capture(window, cx);
        } else if self.cheatsheet.is_some() {
            self.close_cheatsheet(window, cx);
        } else if self.zapping.is_some() {
            self.finish_zap(cx);
        } else if self.cancel_front_modal(window, cx) {
            // A page's dialog or sign-in prompt took the key.
        } else if self.menu.is_some() {
            // A menu can sit over the Peek (opened from the sidebar).
            self.close_menu(window, cx);
        } else if self.passwords_escape(cx) {
            // The save-password popover or a login suggestions dropdown.
        } else if self.boost_editor.is_some() {
            self.boost_editor = None;
        } else if let Some(editor) = self.space_editor.clone() {
            // Esc with the focus outside the editor's fields still cancels
            // a space being created, as Esc inside them does.
            editor.update(cx, |e, cx| e.cancel(cx));
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
    pub(crate) fn render_wash(&self, radius: Pixels, cx: &App) -> Option<AnyElement> {
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
            self.render_page_elsewhere(cx)
                .unwrap_or_else(|| div().size_full().into_any_element())
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
        crate::windows::use_theme(self.window_handle, cx);
        self.sync_view_visibility(cx);
        let t = cx.tokens().clone();
        let fullscreen = self.selected_live().is_some_and(|l| l.fullscreen);
        // Little Mori has a compact header instead of the sidebar.
        let little = self.kind == crate::windows::WindowKind::Little;
        let docked = self.settings.docks_sidebar() && !fullscreen && !little;
        let motion = self.sidebar_fx.motion.filter(|_| !fullscreen && !little);
        // The layout takes its final shape at once (the page is resized
        // once, not on every frame of the slide); the sidebar slides over it.
        let show_sidebar = docked;
        let left = self.settings.sidebar_position == SidebarPosition::Left;
        let inset = if fullscreen { px(0.) } else { card_inset(&self.settings) };
        let loading = !self.settings_open
            && !fullscreen
            && (self.selected_live().is_some_and(|l| l.loading)
                || self.split.and_then(|id| self.live.get(&id)).is_some_and(|l| l.loading));

        let sidebar_width = px(self.sidebar_drag.unwrap_or(self.settings.clamped_sidebar_width()));
        let sidebar_column = show_sidebar.then(|| {
            if motion.is_some() {
                // Sliding in: the room is made, the sidebar arrives on top.
                return div().w(sidebar_width + px(8.)).h_full().flex_none().into_any_element();
            }
            let sidebar = self.render_sidebar(false, window, cx).into_any_element();
            let handle = self.render_resize_handle(left, cx).into_any_element();
            let column = h_flex().h_full().flex_none().items_stretch();
            if left { column.child(sidebar).child(handle) } else { column.child(handle).child(sidebar) }.into_any_element()
        });
        // The docked sidebar sliding in or out, opaque over the page.
        let sliding = motion.map(|(showing, serial)| {
            let sidebar = self.render_sidebar(false, window, cx);
            let travel = sidebar_width + px(8.);
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .w(sidebar_width)
                .when(left, |s| s.left_0())
                .when(!left, |s| s.right_0())
                .bg(t.sidebar)
                .overflow_hidden()
                .occlude()
                .children(self.render_wash(px(0.), cx))
                .child(sidebar)
                .with_animation(
                    ("sidebar-motion", serial),
                    Animation::new(SIDEBAR_MOTION).with_easing(ease_in_out),
                    move |el, delta| {
                        let shown = if showing { delta } else { 1. - delta };
                        let offset = -(travel * (1. - shown));
                        if left { el.left(offset) } else { el.right(offset) }
                    },
                )
        });
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
                .children(sidebar_column)
                .child(web_column)
                .children(assistant.map(|a| a.into_any_element()));
        } else {
            row = row
                .children(assistant.map(|a| a.into_any_element()))
                .child(web_column)
                .children(sidebar_column);
        }
        let row = if little && !fullscreen {
            v_flex()
                .size_full()
                .child(self.render_little_header(cx))
                .child(row.flex_1().min_h_0())
                .into_any_element()
        } else {
            row.into_any_element()
        };

        // Hidden sidebar (and compact mode): touching the window edge slides
        // it in as a card over the page (see `track_sidebar_peek`).
        let floating = (self.sidebar_peek && !show_sidebar && motion.is_none() && !fullscreen && !little).then(|| {
            let sidebar = self.render_sidebar(true, window, cx);
            let leaving = self.sidebar_fx.peek_leaving;
            let serial = self.sidebar_fx.serial;
            let travel = px(self.settings.clamped_sidebar_width()) + px(16.);
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
                .with_animation(
                    (if leaving { "sidebar-peek-out" } else { "sidebar-peek-in" }, serial),
                    Animation::new(if leaving { SIDEBAR_PEEK_OUT } else { SIDEBAR_PEEK_IN })
                        .with_easing(ease_out_quint()),
                    move |el, delta| {
                        let shown = if leaving { 1. - delta } else { delta };
                        let offset = px(4.) - travel * (1. - shown);
                        let el = el.opacity(0.4 + 0.6 * shown);
                        if left { el.left(offset) } else { el.right(offset) }
                    },
                )
        });

        let launcher = self.launcher.clone();
        let passwords = self.render_password_overlays(window, cx);
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
        // The command bar opened over the cheatsheet or capture mode (Ctrl+T,
        // Ctrl+L) replaces them; otherwise they'd sit on top of it.
        if self.launcher.is_some() {
            self.cheatsheet = None;
            self.capture = None;
        }
        let capture = self.render_capture(cx);
        let tab_hover = self.render_tab_hover(window, cx);
        let cheatsheet = self.render_cheatsheet();
        let tab_switcher = self.render_tab_switcher(window, cx);
        let onboarding = self.render_onboarding(window, cx);

        div()
            .id("mori-root")
            // The welcome takes the whole window: browser shortcuts would
            // act on things hidden behind it, so they're off until it ends.
            .key_context(if self.onboarding.is_some() { crate::ui::onboarding::CONTEXT } else { CONTEXT })
            .track_focus(&self.focus_handle)
            .capture_any_mouse_down(cx.listener(|this, ev: &MouseDownEvent, _, _| {
                if ev.button == MouseButton::Left {
                    this.click_at = Some(std::time::Instant::now());
                    if ev.modifiers.alt {
                        this.alt_click_at = this.click_at;
                    }
                }
            }))
            .capture_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                // Enter or Space can follow a focused link, like a click.
                if matches!(ev.keystroke.key.as_str(), "enter" | "space") {
                    this.click_at = Some(std::time::Instant::now());
                }
                if this.onboarding.is_some() {
                    this.on_onboarding_key(ev, window, cx);
                } else {
                    this.on_switcher_key(ev, window, cx);
                }
            }))
            .on_modifiers_changed(cx.listener(|this, ev: &ModifiersChangedEvent, window, cx| {
                this.on_switcher_modifiers(ev, window, cx)
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
            .on_action(cx.listener(|this, _: &SwitchRecentTab, window, cx| this.switch_recent_tab(1, window, cx)))
            .on_action(cx.listener(|this, _: &SwitchRecentTabBack, window, cx| {
                this.switch_recent_tab(-1, window, cx)
            }))
            .on_action(cx.listener(|this, _: &Capture, window, cx| this.start_capture(window, cx)))
            .on_action(cx.listener(|this, _: &ShowShortcuts, window, cx| this.toggle_cheatsheet(window, cx)))
            .on_action(cx.listener(|this, _: &ToggleCompactMode, _, cx| this.toggle_compact_mode(cx)))
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
            .on_action(cx.listener(|_, _: &CloseWindow, window, _| window.remove_window()))
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
            .children(sliding)
            .children(floating)
            .children(capture)
            .children(panel)
            .children(peek)
            .children(popup)
            .children(overlay_toasts)
            .children(passwords)
            .children(boost.map(|b| b.into_any_element()))
            .children(space_editor.map(|e| e.into_any_element()))
            .children(tab_hover)
            .children(launcher.map(|l| l.into_any_element()))
            .children(menu)
            .children(cheatsheet)
            .children(tab_switcher)
            .children(onboarding)
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
