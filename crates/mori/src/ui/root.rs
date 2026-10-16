//! Window layout: the unified chrome surface, sidebar, web card, assistant
//! panel, and every overlay, plus action routing.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use mori_core::settings::SidebarPosition;

use crate::{
    actions::*,
    browser::{Browser, Panel},
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
        } else if self.boost_editor.is_some() {
            self.boost_editor = None;
        } else if self.peek.is_some() {
            self.close_peek(window, cx);
        } else if self.menu.is_some() {
            self.close_menu(window, cx);
        } else if self.launcher.is_some() {
            self.close_launcher(window, cx);
        } else if self.rename.is_some() {
            self.cancel_rename(window, cx);
        } else if self.panel.is_some() {
            self.panel = None;
        } else if self.find_bar.is_some() {
            self.close_find(window, cx);
        } else if self.settings_open {
            self.settings_open = false;
            self.focus_selected_page(window, cx);
        } else if !self.dialogs.is_empty() {
            self.resolve_dialog(false, window, cx);
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

    fn render_wash(&self, cx: &App) -> Option<AnyElement> {
        let t = cx.tokens();
        let wash = t.wash.clone()?;
        let grain = t.grain.clone();
        let opacity = t.gradient.clamped_opacity();
        Some(
            canvas(
                |_, _, _| (),
                move |bounds, _, window, _| {
                    let _ = window.paint_image(bounds, bounds, Corners::default(), wash, 0, false);
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
        } else if Browser::is_internal(&url) {
            self.render_new_tab_page(cx).into_any_element()
        } else {
            div().size_full().into_any_element()
        };

        let live = self.selected_live();
        let loading = live.is_some_and(|l| l.loading) && !self.settings_open;
        let error = live.and_then(|l| l.error.clone()).filter(|_| !self.settings_open);
        let status = live.and_then(|l| l.status.clone()).filter(|_| !self.settings_open);
        let fullscreen = live.is_some_and(|l| l.fullscreen);

        let mut card = div()
            .id("web-card")
            .relative()
            .size_full()
            .bg(t.card)
            .when(!fullscreen, |s| {
                s.rounded(metrics::RADIUS)
                    .border_1()
                    .border_color(t.border.opacity(0.7))
                    .shadow(t.card_shadow())
            })
            .child(content);

        if loading {
            card = card.child(loading_bar(cx));
        }
        if let Some(status) = status {
            card = card.child(
                div()
                    .absolute()
                    .bottom(px(8.))
                    .left(px(8.))
                    .max_w(px(480.))
                    .px_2()
                    .py_1()
                    .rounded(metrics::RADIUS_MD)
                    .bg(t.popover)
                    .border_1()
                    .border_color(t.border.opacity(0.6))
                    .text_size(metrics::TEXT_SMALL)
                    .text_color(t.muted_foreground)
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
        if let Some(dialog) = self.render_js_dialog(cx) {
            card = card.child(dialog);
        }
        if let Some(prompt) = self.render_permission(cx) {
            card = card.child(prompt);
        }
        card.drag_over::<crate::ui::sidebar::DraggedTab>(move |s, _, _, _| {
            s.border_2().border_color(t.primary.opacity(0.7))
        })
        .on_drop(cx.listener(|this, d: &crate::ui::sidebar::DraggedTab, window, cx| {
            this.open_split(d.id, window, cx)
        }))
        .into_any_element()
    }

    /// The second page of a split view, with its own close button.
    fn render_split_card(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let id = self.split?;
        let live = self.live.get(&id)?;
        let t = cx.tokens().clone();
        let loading = live.loading;
        Some(
            div()
                .id("split-card")
                .relative()
                .flex_1()
                .min_w_0()
                .h_full()
                .bg(t.card)
                .rounded(metrics::RADIUS)
                .border_1()
                .border_color(t.border.opacity(0.7))
                .shadow(t.card_shadow())
                .child(live.surface.clone())
                .when(loading, |s| s.child(loading_bar(cx)))
                .child(
                    div()
                        .id("close-split")
                        .absolute()
                        .top(px(10.))
                        .right(px(10.))
                        .size(px(22.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(t.popover.opacity(0.9))
                        .border_1()
                        .border_color(t.border.opacity(0.6))
                        .cursor_pointer()
                        .child(icon("x", px(10.), t.foreground))
                        .on_click(cx.listener(|this, _, _, cx| this.close_split(cx))),
                )
                .into_any_element(),
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
            .rounded(metrics::RADIUS)
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

    fn render_toasts(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.toasts.is_empty() {
            return None;
        }
        let t = cx.tokens().clone();
        let toasts: Vec<AnyElement> = self
            .toasts
            .iter()
            .map(|toast| {
                use crate::browser::ToastStyle as S;
                let (ic, color) = match toast.style {
                    S::Info => ("info", t.status_info),
                    S::Success => ("circle-check", t.status_success),
                    S::Warning => ("triangle-alert", t.status_warning),
                    S::Error => ("circle-x", t.destructive),
                };
                let id = toast.id;
                let action = toast.action.clone();
                h_flex()
                    .id(("toast", id as usize))
                    .gap_2()
                    .px(px(14.))
                    .py(px(10.))
                    .rounded(metrics::RADIUS_POPOVER)
                    .bg(t.popover)
                    .border_1()
                    .border_color(t.border.opacity(0.6))
                    .shadow(t.popover_shadow())
                    .text_size(metrics::TEXT_BASE)
                    .text_color(t.foreground)
                    .cursor_pointer()
                    .child(icon(ic, px(14.), color))
                    .child(toast.text.clone())
                    .when_some(action, |s, (label, action)| {
                        s.child(
                            div()
                                .id(("toast-action", id as usize))
                                .ml_2()
                                .px_2()
                                .py_0p5()
                                .rounded(metrics::RADIUS_MD)
                                .bg(t.primary.opacity(0.15))
                                .text_color(t.primary)
                                .font_weight(FontWeight::MEDIUM)
                                .hover(|s| s.bg(t.primary.opacity(0.25)))
                                .child(label)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.run_toast_action(id, action.clone(), window, cx);
                                })),
                        )
                    })
                    .on_click(cx.listener(move |this, _, _, cx| this.dismiss_toast(id, cx)))
                    .into_any_element()
            })
            .collect();
        Some(
            v_flex()
                .absolute()
                .bottom(px(24.))
                .left_0()
                .right_0()
                .items_center()
                .gap_2()
                .children(toasts)
                .into_any_element(),
        )
    }
}

fn loading_bar(cx: &App) -> impl IntoElement {
    let t = cx.tokens();
    let color = t.muted_foreground;
    div()
        .absolute()
        .bottom(px(1.))
        .left(px(10.))
        .right(px(10.))
        .h(px(2.))
        .overflow_hidden()
        .child(
            div()
                .h_full()
                .w(relative(0.28))
                .rounded_full()
                .bg(linear_gradient(
                    90.,
                    linear_color_stop(color.opacity(0.0), 0.),
                    linear_color_stop(color.opacity(0.7), 1.),
                ))
                .with_animation(
                    "loading-sweep",
                    Animation::new(std::time::Duration::from_millis(1100))
                        .repeat()
                        .with_easing(ease_in_out),
                    |el, delta| el.ml(relative(delta * 0.72)),
                ),
        )
}

impl Render for Browser {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens().clone();
        let fullscreen = self.selected_live().is_some_and(|l| l.fullscreen);
        let show_sidebar = self.settings.sidebar_visible && !fullscreen;
        let left = self.settings.sidebar_position == SidebarPosition::Left;
        let inset = if fullscreen { px(0.) } else { metrics::CARD_INSET };

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

        let web_column = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .pt(inset)
            .pb(inset)
            .pl(if show_sidebar && left { px(0.) } else { inset })
            .pr(if show_sidebar && !left { px(0.) } else { inset })
            .flex()
            .gap(metrics::CARD_INSET)
            .child(div().flex_1().min_w_0().h_full().child(card))
            .children(split_card);

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

        // Hidden sidebar: hovering the window edge reveals it as a card.
        let edge_peek = (!show_sidebar && !fullscreen).then(|| {
            div()
                .id("sidebar-edge")
                .absolute()
                .top_0()
                .bottom_0()
                .w(px(12.))
                .when(left, |s| s.left_0())
                .when(!left, |s| s.right_0())
                .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                    if *hovered {
                        this.sidebar_peek = true;
                        cx.notify();
                    }
                }))
        });
        let floating = (self.sidebar_peek && !show_sidebar && !fullscreen).then(|| {
            let sidebar = self.render_sidebar(true, window, cx);
            div()
                .id("sidebar-peek")
                .absolute()
                .top(px(8.))
                .bottom(px(8.))
                .when(left, |s| s.left(px(8.)))
                .when(!left, |s| s.right(px(8.)))
                .rounded(metrics::RADIUS)
                .bg(t.sidebar.opacity(if t.dark { 0.92 } else { 0.95 }))
                .border_1()
                .border_color(t.sidebar_border.opacity(0.7))
                .shadow(t.overlay_shadow())
                .occlude()
                .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                    if !*hovered && this.menu.is_none() {
                        this.sidebar_peek = false;
                        cx.notify();
                    }
                }))
                .child(sidebar)
        });

        let launcher = self.launcher.clone();
        let menu = self.render_menu(window, cx);
        let panel = self.render_panel(window, cx);
        let peek = self.render_peek(window, cx);
        let boost = self.boost_editor.clone();
        let toasts = self.render_toasts(cx);

        div()
            .id("mori-root")
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .relative()
            .size_full()
            .bg(t.sidebar)
            .text_color(t.foreground)
            .font_family(".SystemUIFont")
            .text_size(metrics::TEXT_BASE)
            .on_action(cx.listener(|this, _: &NewTab, window, cx| {
                this.open_launcher(LauncherMode::NewTab, window, cx)
            }))
            .on_action(cx.listener(|this, _: &OpenLocation, window, cx| {
                this.open_launcher(LauncherMode::Address, window, cx)
            }))
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| {
                if let Some(id) = this.selected() {
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
            .on_action(cx.listener(|this, _: &OpenSettings, _, cx| {
                this.settings_open = !this.settings_open;
                cx.notify();
            }))
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
            .on_action(cx.listener(|this, _: &ToggleDevTools, _, _| this.toggle_devtools()))
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
            .children(self.render_wash(cx))
            .child(row)
            .children(edge_peek)
            .children(floating)
            .children(panel)
            .children(peek)
            .children(boost.map(|b| b.into_any_element()))
            .children(launcher.map(|l| l.into_any_element()))
            .children(menu)
            .children(toasts)
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

struct EmptyView;

impl Render for EmptyView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}
