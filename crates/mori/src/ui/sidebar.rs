//! The sidebar: navigation, address pill, the favorites grid shared by all
//! spaces, the space's pinned rows and folders, the divider with "Clear",
//! today's tabs, and the space switcher.

use std::{sync::Arc, time::Duration};

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{component::input::Input, *};
use mori_core::{
    omnibox,
    util::host_of,
    workspace::{Folder, FolderId, Item, Slot, SpaceId, TabId},
};

use crate::{
    browser::{Browser, MenuKind, OpenMenu, Panel, RenameTarget},
    theme::{ActiveTokens, metrics},
    ui::{
        components::{favicon, h_flex, icon, icon_button, v_flex},
        launcher::LauncherMode,
    },
};

/// Payload while a tab (or a multi-selection) is being dragged.
#[derive(Clone)]
pub struct DraggedTab {
    /// The tab under the pointer.
    pub id: TabId,
    /// Every tab moving, in sidebar order (just `id` unless the dragged
    /// tab is part of a multi-selection).
    pub ids: Vec<TabId>,
    pub title: SharedString,
    pub favicon: Option<Arc<RenderImage>>,
    /// Where it was picked up, to show which side of a row it will land on.
    pub origin: Option<(SpaceId, Slot)>,
}

impl DraggedTab {
    /// Whether dropping onto `target` moves the tab down (or right) within
    /// its own list, so it lands after the target rather than before.
    fn lands_after(&self, space: SpaceId, target: Slot) -> bool {
        lands_after(self.origin, space, target)
    }
}

fn lands_after(origin: Option<(SpaceId, Slot)>, space: SpaceId, target: Slot) -> bool {
    origin.is_some_and(|(s, from)| {
        (s == space || from.is_favorite()) && from.same_list(&target) && from.index() < target.index()
    })
}

impl Render for DraggedTab {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens().clone();
        let count = self.ids.len();
        drag_chip(cx)
            .child(favicon(self.favicon.clone(), metrics::FAVICON, false, cx))
            .child(div().truncate().child(self.title.clone()))
            .when(count > 1, |s| {
                s.child(
                    div()
                        .flex_none()
                        .px(px(6.))
                        .rounded_full()
                        .bg(t.primary)
                        .text_color(t.primary_foreground)
                        .text_size(metrics::TEXT_SMALL)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(count.to_string()),
                )
            })
    }
}

/// Payload while a folder is being dragged.
#[derive(Clone)]
pub struct DraggedFolder {
    pub id: FolderId,
    pub name: SharedString,
    pub origin: (SpaceId, Slot),
}

impl Render for DraggedFolder {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens().clone();
        drag_chip(cx)
            .child(svg().path("mori/folder-filled.svg").size(px(18.)).flex_none().text_color(t.primary))
            .child(div().truncate().font_weight(FontWeight::SEMIBOLD).child(self.name.clone()))
    }
}

/// Payload while a space chip is being dragged in the switcher.
#[derive(Clone)]
pub struct DraggedSpace {
    pub id: SpaceId,
    pub symbol: SharedString,
    pub index: usize,
}

impl Render for DraggedSpace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens().clone();
        div()
            .size(px(28.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(7.))
            .bg(t.popover)
            .border_1()
            .border_color(t.popover_border())
            .shadow(t.popover_shadow())
            .child(space_glyph(&self.symbol, px(14.), t.foreground))
    }
}

fn drag_chip(cx: &App) -> Div {
    let t = cx.tokens();
    h_flex()
        .h(metrics::ROW_HEIGHT)
        .pl(px(7.))
        .pr(px(10.))
        .gap(px(10.))
        .max_w(px(220.))
        .rounded(metrics::ROW_RADIUS)
        .bg(t.popover)
        .border_1()
        .border_color(t.popover_border())
        .shadow(t.popover_shadow())
        .text_size(crate::theme::ui_font().size)
        .text_color(t.foreground)
}

/// Where a dropped tab or folder lands.
#[derive(Clone, Copy, Debug)]
pub enum DropTarget {
    /// Before the given position.
    Slot(SpaceId, Slot),
    /// Onto a row or tile: it takes that row's place.
    Onto(SpaceId, Slot),
    /// Inside a folder, at its end.
    FolderEnd(SpaceId, FolderId),
    LooseEnd(SpaceId),
    /// The end of the space's pinned rows.
    PinnedEnd(SpaceId),
    FavoritesEnd,
    /// Another space (a switcher chip).
    Space(SpaceId),
}

pub use mori_core::workspace::DEFAULT_SPACE_SYMBOL;

/// Space glyphs offered first in the space editor (Lucide icon names); its
/// search covers the whole Lucide set.
pub const SPACE_SYMBOLS: [&str; 48] = [
    "circle", "house", "briefcase", "code", "book-open", "music", "gamepad-2", "palette",
    "coffee", "leaf", "heart", "star", "flame", "zap", "globe", "graduation-cap", "camera",
    "plane", "shopping-bag", "dumbbell", "sparkles", "moon", "sun", "folder", "rocket",
    "terminal", "film", "newspaper", "pen-tool", "flask-conical", "landmark", "wallet",
    "car", "bike", "mountain", "tree-pine", "flower-2", "cat", "dog", "baby", "users",
    "message-circle", "mail", "calendar", "chart-line", "cpu", "trophy", "gift",
];

/// Emoji offered in the space editor next to a field for any other.
pub const SPACE_EMOJI: [&str; 24] = [
    "😀", "😎", "🤓", "🧠", "💼", "🏠", "💻", "📚", "🎨", "🎵", "🎮", "⚽", "🍳", "☕", "🌱", "🌊",
    "🔥", "⭐", "❤️", "🚀", "✈️", "💰", "🛒", "🐱",
];

/// Folder glyphs offered in a folder's menu.
pub const FOLDER_SYMBOLS: [&str; 12] = [
    "folder", "star", "heart", "briefcase", "code", "book-open", "music", "palette", "bookmark", "flag",
    "zap", "archive",
];

/// How far a horizontal two-finger swipe must travel to switch spaces.
const SWIPE_DISTANCE: f32 = 90.;
/// A pause this long ends a swipe gesture (Wayland reports no end).
const SWIPE_GAP: Duration = Duration::from_millis(220);
/// How long the sidebar contents slide when the space changes.
const SPACE_SLIDE: Duration = Duration::from_millis(240);

/// A horizontal swipe over the sidebar in progress.
#[derive(Default)]
pub struct SidebarSwipe {
    distance: f32,
    last: Option<std::time::Instant>,
    /// This gesture already switched spaces; the rest of it is ignored.
    spent: bool,
}

/// The latest space change, for the sidebar's slide: a serial that keys
/// the animation and the direction the new space comes from (1 = from the
/// right, the next space).
#[derive(Clone, Copy, Default)]
pub struct SpaceSlide {
    pub serial: usize,
    pub direction: f32,
}

impl Browser {
    fn favicon_for(&mut self, id: TabId) -> Option<Arc<RenderImage>> {
        let host = self.ws.tab(id).and_then(|t| host_of(&t.url))?;
        self.favicons.get(&host)
    }

    /// What dragging `id` moves: the whole multi-selection when it is part
    /// of one, else just the tab.
    fn drag_group(&self, id: TabId) -> Vec<TabId> {
        if self.multi_select.len() > 1 && self.multi_select.contains(&id) {
            self.ws.in_sidebar_order(&self.multi_select)
        } else {
            vec![id]
        }
    }

    pub fn drop_tab(&mut self, dragged: &DraggedTab, target: DropTarget, window: &mut Window, cx: &mut Context<Self>) {
        let ids = if dragged.ids.is_empty() { vec![dragged.id] } else { dragged.ids.clone() };
        let ok = match target {
            DropTarget::Slot(space, slot) => self.ws.move_tabs(&ids, space, slot),
            DropTarget::Onto(space, slot) => self.ws.move_tabs_onto(&ids, dragged.id, space, slot),
            DropTarget::FolderEnd(space, folder) => self.ws.move_tabs(&ids, space, Slot::Folder(folder, usize::MAX)),
            DropTarget::LooseEnd(space) => self.ws.move_tabs(&ids, space, Slot::Loose(usize::MAX)),
            DropTarget::PinnedEnd(space) => self.ws.move_tabs(&ids, space, Slot::Pinned(usize::MAX)),
            DropTarget::FavoritesEnd => {
                let space = self.ws.active_space;
                self.ws.move_tabs(&ids, space, Slot::Favorite(usize::MAX))
            }
            DropTarget::Space(space) => {
                self.move_tabs_to_space(ids, space, window, cx);
                true
            }
        };
        if !ok && matches!(target, DropTarget::FavoritesEnd | DropTarget::Onto(_, Slot::Favorite(_))) {
            self.toast(
                format!("Favorites hold up to {} tabs", mori_core::workspace::MAX_FAVORITES),
                crate::browser::ToastStyle::Info,
                None,
                cx,
            );
        }
        self.multi_select.clear();
        self.schedule_save(cx);
        cx.notify();
    }

    pub fn drop_folder(&mut self, dragged: &DraggedFolder, target: DropTarget, window: &mut Window, cx: &mut Context<Self>) {
        let folder = dragged.id;
        match target {
            DropTarget::Slot(space, slot) => {
                self.ws.move_folder(folder, space, slot);
            }
            DropTarget::Onto(space, slot) => {
                let after = lands_after(Some(dragged.origin), space, slot);
                let slot = if after { slot.with_index(slot.index() + 1) } else { slot };
                self.ws.move_folder(folder, space, slot);
            }
            DropTarget::FolderEnd(space, into) => {
                // Into itself or a folder inside it: nothing to say.
                let own = self
                    .ws
                    .space(space)
                    .is_some_and(|s| s.folder_trail(into).iter().any(|f| f.id == folder));
                if !self.ws.move_folder(folder, space, Slot::Folder(into, usize::MAX)) && !own {
                    self.toast(
                        format!("Folders nest up to {} levels", mori_core::workspace::MAX_FOLDER_DEPTH),
                        crate::browser::ToastStyle::Info,
                        None,
                        cx,
                    );
                }
            }
            DropTarget::PinnedEnd(space) => {
                self.ws.move_folder(folder, space, Slot::Pinned(usize::MAX));
            }
            DropTarget::Space(space) => self.move_folder_to_space(folder, space, window, cx),
            DropTarget::LooseEnd(_) | DropTarget::FavoritesEnd => {}
        }
        self.schedule_save(cx);
        cx.notify();
    }

    /// Horizontal two-finger swipes over the sidebar switch spaces, as in
    /// Arc: content moving left brings in the next space.
    fn on_sidebar_scroll(&mut self, ev: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        let ScrollDelta::Pixels(delta) = ev.delta else { return };
        let (dx, dy) = (f32::from(delta.x), f32::from(delta.y));
        let now = std::time::Instant::now();
        let swipe = &mut self.sidebar_swipe;
        if swipe.last.is_none_or(|at| now.duration_since(at) > SWIPE_GAP) {
            *swipe = SidebarSwipe::default();
        }
        swipe.last = Some(now);
        if swipe.spent || dx.abs() <= dy.abs() {
            return;
        }
        swipe.distance += dx;
        if swipe.distance.abs() < SWIPE_DISTANCE {
            return;
        }
        let step = if swipe.distance < 0. { 1 } else { -1 };
        swipe.spent = true;
        self.step_space(step, window, cx);
    }

    /// Switch to the space `delta` steps away in the switcher (wrapping).
    pub fn step_space(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(space) = self.ws.space_neighbor(delta) {
            self.switch_space(space, window, cx);
            // Wrapping around still slides the way the user went.
            self.space_slide.direction = delta.signum() as f32;
        }
    }

    pub fn render_sidebar(&mut self, floating: bool, window: &mut Window, cx: &mut Context<Self>) -> Stateful<Div> {
        let t = cx.tokens().clone();
        let font = crate::theme::ui_font();
        let width = px(self.sidebar_drag.unwrap_or(self.settings.clamped_sidebar_width()));
        let left = self.settings.sidebar_position == mori_core::settings::SidebarPosition::Left;
        // Content sits 7px from the window edge and runs up to the gutter
        // before the web card (Zen); a floating sidebar is inset both sides.
        let (outer, inner) = if floating {
            (metrics::SIDEBAR_INSET, metrics::SIDEBAR_INSET)
        } else {
            (metrics::SIDEBAR_INSET, px(0.))
        };
        let (pl, pr) = if left { (outer, inner) } else { (inner, outer) };

        let mut contents = v_flex()
            .id("sidebar-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            // A sideways swipe switches spaces instead of scrolling the list.
            .restrict_scroll_to_axis()
            .pl(pl + metrics::ROW_INSET)
            .pr(pr + metrics::ROW_INSET)
            .child(self.render_space_header(cx))
            .child(self.render_pinned_section(window, cx))
            .child(self.render_divider(cx))
            .child(self.render_loose(window, cx));
        // The space's own contents slide in from the side the new space
        // sits on (the favorites above stay put, as in Arc).
        let slide = self.space_slide;
        let animated: AnyElement = if slide.serial == 0 {
            contents.into_any_element()
        } else {
            contents = contents.relative();
            contents
                .with_animation(
                    ("space-slide", slide.serial),
                    Animation::new(SPACE_SLIDE).with_easing(ease_out_quint()),
                    move |el, delta| el.left(px(slide.direction * 36. * (1. - delta))).opacity(0.25 + 0.75 * delta),
                )
                .into_any_element()
        };

        v_flex()
            .id("sidebar")
            .group("sidebar")
            .w(width)
            .h_full()
            .flex_none()
            .text_color(t.sidebar_foreground)
            .text_size(font.size)
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                    this.menu = Some(OpenMenu {
                        kind: MenuKind::Sidebar,
                        position: ev.position,
                    });
                    cx.notify();
                }),
            )
            .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, window, cx| this.on_sidebar_scroll(ev, window, cx)))
            .child(self.render_header(left, window, cx).pl(pl).pr(pr))
            .child(div().flex_none().pl(pl).pr(pr).child(self.render_favorites(cx)))
            .child(animated)
            .children(self.render_media_strip(cx))
            // The bar's own -4/-3px margins give back 7px of the padding.
            .child(self.render_bottom_bar(f32::from(width - pl - pr) + 7., cx).pl(pl).pr(pr))
    }

    fn render_header(&mut self, left: bool, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = cx.tokens().clone();
        let live = self.active_live();
        let (can_back, can_fwd, loading) = live
            .map(|l| (l.can_go_back, l.can_go_forward, l.loading))
            .unwrap_or((false, false, false));
        let has_downloads = !self.downloads.is_empty();
        let active_downloads = self.downloads.active_count();
        let download_fraction = self.downloads.active_fraction();
        let button = metrics::TOOLBAR_BUTTON;

        // Zen's toolbar: app menu and sidebar toggle on the left, navigation
        // on the right; 30px buttons with 16px glyphs and no gaps.
        let nav = h_flex()
            .h(button)
            .ml(px(-4.))
            .mr(px(-3.))
            .child(
                icon_button("app-menu", "ellipsis", button, true, cx)
                    .relative()
                    .child(crate::ui::menus::anchor_probe(crate::ui::menus::APP_MENU_ANCHOR))
                    .on_click(cx.listener(
                    |this, ev: &ClickEvent, _, cx| {
                        this.menu = Some(OpenMenu {
                            kind: MenuKind::Sidebar,
                            position: ev.position(),
                        });
                        cx.notify();
                    },
                )),
            )
            .child(
                icon_button("toggle-sidebar", if left { "panel-left" } else { "panel-right" }, button, true, cx)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx))),
            )
            .child(window_drag_area("window-drag"))
            // A ring shows the progress of running downloads.
            .when(has_downloads, |s| {
                s.child(
                    div()
                        .relative()
                        .child(
                            icon_button("downloads", "download", button, true, cx).on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.panel = (this.panel != Some(Panel::Downloads))
                                        .then_some(Panel::Downloads);
                                    cx.notify();
                                }),
                            ),
                        )
                        .child(crate::ui::panels::download_ring(
                            download_fraction,
                            active_downloads > 0,
                            t.status_info,
                            t.sidebar_foreground.opacity(0.14),
                        )),
                )
            })
            .child(
                icon_button("back", "arrow-left", button, can_back, cx).on_click(cx.listener(
                    |this, _, _, _| {
                        if let Some(v) = this.selected_view() {
                            v.go_back();
                        }
                    },
                )),
            )
            .child(
                icon_button("forward", "arrow-right", button, can_fwd, cx).on_click(cx.listener(
                    |this, _, _, _| {
                        if let Some(v) = this.selected_view() {
                            v.go_forward();
                        }
                    },
                )),
            )
            .child(
                icon_button("reload", if loading { "x" } else { "rotate-cw" }, button, live.is_some(), cx)
                    .on_click(cx.listener(move |this, _, _, _| {
                        if let Some(v) = this.selected_view() {
                            if loading {
                                v.stop();
                            } else {
                                v.reload(false);
                            }
                        }
                    })),
            );

        // The window buttons don't fit next to the toolbar at Zen's sidebar
        // width, so they get a row of their own above it.
        let controls = window_controls(window, cx).map(|controls| {
            h_flex()
                .h(px(24.))
                .mr(px(-3.))
                .child(window_drag_area("window-drag-controls"))
                .child(controls)
        });
        v_flex()
            .pt(px(6.))
            .pb(px(10.))
            .gap(px(7.))
            .children(controls)
            .child(nav)
            .child(self.render_omnibox(cx))
    }

    /// Ctrl+T's bar is open: the sidebar shows the tab being created as
    /// the selected one, and no other.
    fn creating_tab(&self, cx: &App) -> bool {
        self.launcher
            .as_ref()
            .is_some_and(|l| l.read(cx).mode() == LauncherMode::NewTab)
    }

    fn render_omnibox(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.tokens().clone();
        let font = crate::theme::ui_font();
        // Ctrl+T's bar stands for a tab that has no address yet.
        let record = self
            .active_tab()
            .filter(|_| !self.creating_tab(cx))
            .and_then(|id| self.ws.tab(id))
            .cloned();
        let url = record.as_ref().map(|r| r.url.clone()).unwrap_or_default();
        let host = omnibox::display_host(&url);
        let is_web = url.starts_with("http://") || url.starts_with("https://");
        let bookmarked = is_web && self.bookmarks.contains(&url);
        let reader = self.active_live().is_some_and(|l| l.reader);
        let muted = t.sidebar_foreground.opacity(if t.dark { 0.6 } else { 0.55 });
        let zoom = self.selected_zoom_level();
        let zoom_label = (is_web && !mori_core::zoom::is_default(zoom))
            .then(|| format!("{}%", mori_core::zoom::percent_for_level(zoom)));

        // Zen shows the bare host; only insecure pages and the empty state
        // get a leading glyph. The lock needs a valid certificate, not just
        // the scheme.
        use mori_core::security::Security;
        let security = match self.active_tab() {
            Some(id) => self.page_security(crate::browser::PageOwner::Tab(id), &url),
            None => Security::Neutral,
        };
        let lead = if matches!(security, Security::NotSecure | Security::CertificateError) {
            Some(icon("triangle-alert", px(14.), t.status_warning))
        } else if host.is_empty() {
            Some(icon("search", px(14.), muted))
        } else if security == Security::Secure {
            // Secure page: a quiet lock that opens site info.
            Some(icon("lock", px(13.), muted))
        } else {
            None
        };
        // Page actions appear on hover, or stay while they show state.
        let action = |id: &'static str, glyph: &'static str, color: Hsla, toggles_reader: bool, cx: &mut Context<Self>| {
            div()
                .id(id)
                .size(px(26.))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .rounded(px(7.))
                .hover(|s| s.bg(t.tab_hover()))
                .cursor_pointer()
                .child(icon(glyph, px(14.), color))
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    if toggles_reader {
                        this.toggle_reader(cx);
                    } else {
                        this.toggle_bookmark(cx);
                    }
                }))
        };
        // The site's zoom when it is not 100%; click to reset.
        let zoom_chip = |id: &'static str, label: String, cx: &mut Context<Self>| {
            div()
                .id(id)
                .flex_none()
                .h(px(18.))
                .px(px(5.))
                .flex()
                .items_center()
                .rounded(metrics::RADIUS_MD)
                .bg(t.foreground.opacity(0.08))
                .hover(|s| s.bg(t.foreground.opacity(0.14)))
                .cursor_pointer()
                .text_size(metrics::TEXT_SMALL)
                .text_color(t.sidebar_foreground.opacity(0.75))
                .child(label)
                .on_click(cx.listener(|this, _, _, cx| {
                    cx.stop_propagation();
                    this.zoom(0.0, cx);
                }))
        };
        let reader_color = if reader { t.primary } else { muted };
        let pill_backing = t.opaque_on_sidebar(t.tile_rest());
        let star_color = if bookmarked { t.status_warning } else { muted };

        let mut pill = h_flex()
            .id("omnibox")
            .group("omnibox")
            .relative()
            .h(metrics::PILL_HEIGHT)
            .pl(px(11.))
            .pr(px(4.))
            .gap(px(7.))
            .rounded(metrics::RADIUS)
            .bg(t.tile_rest())
            .cursor_text()
            .on_click(cx.listener(|this, _, window, cx| {
                this.open_launcher(LauncherMode::Address, window, cx)
            }))
            .child(crate::ui::menus::anchor_probe(crate::ui::site_info::OMNIBOX_ANCHOR))
            // The leading icon opens site info (connection, permissions, zoom).
            .when_some(lead, |s, lead| {
                s.child(
                    div()
                        .id("site-info")
                        .flex_none()
                        .size(px(20.))
                        .mx(px(-2.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(metrics::RADIUS_MD)
                        .cursor_pointer()
                        .hover(|s| s.bg(t.ghost_hover()))
                        .child(lead)
                        .when(is_web, |s| {
                            s.on_click(cx.listener(|this, ev: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                this.menu = Some(OpenMenu {
                                    kind: MenuKind::SiteInfo,
                                    position: ev.position(),
                                });
                                cx.notify();
                            }))
                        }),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    // A long address fades out (as Zen's does, and as tab
                    // titles do here) where the pill has a single colour.
                    .map(|s| match pill_backing {
                        Some(_) => s.relative().overflow_hidden().whitespace_nowrap(),
                        None => s.truncate(),
                    })
                    .text_size(font.url_size())
                    .font_weight(FontWeight::MEDIUM)
                    .when(host.is_empty(), |s| s.text_color(muted).child("Search..."))
                    .when(!host.is_empty(), |s| {
                        s.text_color(t.sidebar_foreground.opacity(if t.dark { 0.75 } else { 0.68 }))
                            .child(host)
                    })
                    // Drawn after the text so it covers its end.
                    .when_some(pill_backing, |s, backing| {
                        s.child(div().absolute().top_0().bottom_0().right_0().w(px(18.)).bg(fade_to(backing)))
                    }),
            );
        if !is_web {
            return pill;
        }
        // Hover may only toggle visibility here, never `display`: GPUI
        // panics when an element's display differs between prepaint and
        // paint, which happens whenever the layout shifts under a resting
        // pointer (a menu closing, a row disappearing).
        // The hover overlay covers the zoom chip, so it carries its own copy
        // of the chip at its end: the chip stays put and clickable, and the
        // host keeps the rest of the pill at rest.
        match t.opaque_on_sidebar(t.tile_rest()) {
            // The host keeps the whole pill until hover; the actions then
            // slide in over its end on the pill's own colour.
            Some(backing) => {
                pill = pill
                    .when(reader, |s| s.child(action("reader", "book-open-text", reader_color, true, cx)))
                    .when(bookmarked, |s| s.child(action("bookmark", "star", star_color, false, cx)))
                    .when_some(zoom_label.clone(), |s, label| s.child(zoom_chip("zoom-chip", label, cx)))
                    .child(
                        h_flex()
                            .absolute()
                            .top(px(4.))
                            .bottom(px(4.))
                            .right(px(4.))
                            .invisible()
                            .group_hover("omnibox", |s| s.visible())
                            .child(div().w(px(16.)).h_full().bg(fade_to(backing)))
                            .child(
                                h_flex()
                                    .h_full()
                                    .items_center()
                                    .gap(px(7.))
                                    .bg(backing)
                                    .child(action("reader-hover", "book-open-text", reader_color, true, cx))
                                    .child(action("bookmark-hover", "star", star_color, false, cx))
                                    .when_some(zoom_label, |s, label| {
                                        s.child(zoom_chip("zoom-chip-hover", label, cx))
                                    }),
                            ),
                    );
            }
            // Under a gradient wash the pill has no single colour to cover
            // the host with, so the actions keep their space.
            None => {
                pill = pill
                    .child(
                        action("reader", "book-open-text", reader_color, true, cx)
                            .when(!reader, |s| s.invisible().group_hover("omnibox", |s| s.visible())),
                    )
                    .child(
                        action("bookmark", "star", star_color, false, cx)
                            .when(!bookmarked, |s| s.invisible().group_hover("omnibox", |s| s.visible())),
                    )
                    .when_some(zoom_label, |s, label| s.child(zoom_chip("zoom-chip", label, cx)));
            }
        }
        pill
    }

    fn render_space_header(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.tokens().clone();
        let font = crate::theme::ui_font();
        let space = self.ws.active().clone();
        let renaming = self
            .rename
            .as_ref()
            .filter(|(r, _)| *r == RenameTarget::Space(space.id))
            .map(|(_, i)| i.clone());
        let id = space.id;
        // The default symbol is drawn as a dot in the switcher; like Zen, the
        // label only carries an icon the user picked.
        let custom_symbol = (space.symbol != DEFAULT_SPACE_SYMBOL).then(|| space.symbol.clone());
        h_flex()
            .id("space-header")
            .flex_none()
            .h(px(24.))
            // Zen: the label's cap-top sits 22px under the pill, or 24px
            // under the essentials grid.
            .mt(px(if self.ws.favorites.is_empty() { 5. } else { 11. }))
            .pl(px(7.))
            .gap(px(6.))
            .children(custom_symbol.map(|symbol| space_glyph(&symbol, px(13.), t.muted_foreground)))
            .child(match renaming {
                Some(input) => div()
                    .flex_1()
                    .child(rename_input(&input, font.label_size(), FontWeight::BOLD, cx))
                    .into_any_element(),
                None => div()
                    .flex_1()
                    .truncate()
                    .text_size(font.label_size())
                    .font_weight(FontWeight::BOLD)
                    .text_color(t.muted_foreground)
                    .child(space.name.clone())
                    .into_any_element(),
            })
            .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                if ev.click_count() == 2 {
                    this.begin_rename(RenameTarget::Space(id), window, cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.menu = Some(OpenMenu {
                        kind: MenuKind::Space(id),
                        position: ev.position,
                    });
                    cx.notify();
                }),
            )
    }

    /// The favorites: one Essentials-style grid of tiles that every space
    /// shares.
    fn render_favorites(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.tokens().clone();
        let space_id = self.ws.active_space;
        let favorites = self.ws.favorites.clone();
        if favorites.is_empty() {
            // A slim drop zone so tabs can become favorites by dragging.
            // It takes no room at rest (the space label sits where Zen's
            // does); it opens up while a tab is dragged over it.
            return div()
                .id("favorites-zone")
                .h(px(6.))
                .mb(px(-6.))
                .rounded(metrics::RADIUS)
                .drag_over::<DraggedTab>(move |s, _, _, _| {
                    s.h(px(40.))
                        .bg(t.primary.opacity(0.14))
                        .border_1()
                        .border_dashed()
                        .border_color(t.primary.opacity(0.7))
                })
                .on_drop(cx.listener(move |this, d: &DraggedTab, window, cx| {
                    this.drop_tab(d, DropTarget::FavoritesEnd, window, cx)
                }))
                .into_any_element();
        }
        let selected = self.active_tab().filter(|_| !self.creating_tab(cx));
        // Zen's selected essential: #585858 fill, #bcbcbc ring.
        let (selected_bg, selected_ring) = if t.dark {
            (white().opacity(0.29), white().opacity(0.70))
        } else {
            (white().opacity(0.80), black().opacity(0.15))
        };
        let mut tiles: Vec<AnyElement> = favorites
            .iter()
            .enumerate()
            .map(|(i, id)| {
                let id = *id;
                let fav = self.favicon_for(id);
                let is_selected = selected == Some(id);
                let in_split = !is_selected && self.visible_split_member(id);
                let multi = self.multi_select.contains(&id);
                let loading = self.live.get(&id).is_some_and(|l| l.loading);
                let record = self.ws.tab(id);
                let title: SharedString = record
                    .map(|r| r.display_title().to_string())
                    .unwrap_or_default()
                    .into();
                let host = record.and_then(|r| host_of(&r.url)).unwrap_or_default();
                let glyph = match fav.clone() {
                    Some(img) => favicon(Some(img), metrics::FAVICON, false, cx),
                    None => monogram(&host, cx),
                };
                let slot = Slot::Favorite(i);
                let group = self.drag_group(id);
                div()
                    .id(("favorite", i))
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .h(px(46.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(metrics::RADIUS)
                    .border_2()
                    .border_color(if is_selected { selected_ring } else { transparent_black() })
                    .bg(if is_selected { selected_bg } else { t.tile_rest() })
                    .when(in_split, |s| s.bg(selected_bg.opacity(0.55)))
                    .when(multi, |s| s.border_color(t.primary.opacity(0.8)))
                    .when(!is_selected, |s| s.hover(|s| s.bg(t.tile_hover())))
                    .cursor_pointer()
                    .child(glyph)
                    .when(loading, |s| {
                        s.child(
                            div()
                                .absolute()
                                .bottom(px(5.))
                                .left(px(5.))
                                .size(px(5.))
                                .rounded_full()
                                .bg(t.status_info),
                        )
                    })
                    .tooltip({
                        let title = title.clone();
                        move |window, cx| gpui_kit::component::tooltip::Tooltip::new(title.clone()).build(window, cx)
                    })
                    .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| this.click_tab(id, ev, window, cx)))
                    .on_mouse_down(
                        MouseButton::Middle,
                        cx.listener(move |this, _, window, cx| this.close_tab(id, false, window, cx)),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            this.menu = Some(OpenMenu {
                                kind: MenuKind::Tab(id),
                                position: ev.position,
                            });
                            cx.notify();
                        }),
                    )
                    .on_drag(
                        DraggedTab {
                            id,
                            ids: group,
                            title,
                            favicon: fav,
                            origin: Some((space_id, slot)),
                        },
                        |d, _, _, cx| cx.new(|_| d.clone()),
                    )
                    .drag_over::<DraggedTab>(move |s, d, _, _| {
                        let s = s.border_color(t.primary.opacity(0.8));
                        if d.lands_after(space_id, slot) {
                            s.border_r_4()
                        } else {
                            s.border_l_4()
                        }
                    })
                    .on_drop(cx.listener(move |this, d: &DraggedTab, window, cx| {
                        this.drop_tab(d, DropTarget::Onto(space_id, slot), window, cx)
                    }))
                    .into_any_element()
            })
            .collect();
        // While a tab is dragged, an extra cell after the last tile adds it
        // at the end; drops on the grid's gaps do the same.
        let tab_drag = cx.has_active_drag() && cx.active_drag_cursor_style() != Some(CursorStyle::ResizeColumn);
        // The cell floats over what follows (the space header) instead of
        // pushing it down: every row must stay where the user saw it when
        // the drag began, or drops land one row off. Painted on top and
        // occluding, so the drop goes to it alone.
        let backing = t.opaque_on_sidebar(t.tile_rest());
        let end_cell = (tab_drag && favorites.len() < mori_core::workspace::MAX_FAVORITES).then(|| {
            deferred(
                div()
                    .id("favorites-end")
                    .absolute()
                    .top_full()
                    .left_0()
                    .right_0()
                    .h(px(30.))
                    .occlude()
                    .rounded(metrics::RADIUS)
                    .when_some(backing, |s, bg| s.bg(bg))
                    .border_1()
                    .border_dashed()
                    .border_color(t.sidebar_foreground.opacity(0.18))
                    .drag_over::<DraggedTab>(move |s, _, _, _| {
                        s.bg(t.primary.opacity(0.14)).border_color(t.primary.opacity(0.7))
                    })
                    .on_drop(cx.listener(move |this, d: &DraggedTab, window, cx| {
                        this.drop_tab(d, DropTarget::FavoritesEnd, window, cx)
                    })),
            )
        });
        // Essentials-style grid: rows of up to four tiles that always fill
        // the sidebar's width, balanced so no row has holes.
        let mut rows = v_flex().id("favorites-grid").relative().gap(px(7.)).pb(px(4.));
        for len in mori_core::theme::balanced_rows(tiles.len(), 4) {
            let rest = tiles.split_off(len);
            rows = rows.child(h_flex().gap(px(7.)).children(std::mem::replace(&mut tiles, rest)));
        }
        rows.children(end_cell)
            .on_drop(cx.listener(move |this, d: &DraggedTab, window, cx| {
                this.drop_tab(d, DropTarget::FavoritesEnd, window, cx)
            }))
            .into_any_element()
    }

    /// A click on a tab row or favorite tile: Ctrl adds it to (or takes it
    /// out of) the multi-selection, Shift selects the range from the last
    /// one, a double-click renames, a plain click selects it.
    fn click_tab(&mut self, id: TabId, ev: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        let m = ev.modifiers();
        if m.secondary() {
            if self.multi_select.is_empty()
                && let Some(current) = self.selected().filter(|s| *s != id)
            {
                // The tab on screen is part of what the user is gathering.
                self.multi_select.push(current);
            }
            if let Some(i) = self.multi_select.iter().position(|t| *t == id) {
                self.multi_select.remove(i);
            } else {
                self.multi_select.push(id);
            }
            cx.notify();
            return;
        }
        if m.shift
            && let Some(anchor) = self.multi_select.last().copied().or(self.selected())
        {
            let range = self.ws.visible_range(anchor, id);
            if !range.is_empty() {
                self.multi_select = range;
                cx.notify();
                return;
            }
        }
        if ev.click_count() == 2 && !self.ws.is_favorite(id) {
            self.begin_rename(RenameTarget::Tab(id), window, cx);
            return;
        }
        self.select_tab(id, window, cx);
    }

    /// The space's pinned section: pinned tabs and folders, above the
    /// divider.
    fn render_pinned_section(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let space = self.ws.active().clone();
        let rows = self.render_items(space.id, &space.pinned, None, window, cx);
        // Zen: the first pinned row starts 24px below the label's middle.
        v_flex().gap(metrics::ROW_GAP).when(!rows.is_empty(), |s| s.mt(px(12.))).children(rows)
    }

    /// Rows for one pinned list (the top level or a folder's items). A split
    /// pair whose tabs sit next to each other shows as one grouped row.
    fn render_items(
        &mut self,
        space: SpaceId,
        items: &[Item],
        parent: Option<FolderId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let slot = |i: usize| match parent {
            None => Slot::Pinned(i),
            Some(f) => Slot::Folder(f, i),
        };
        let mut rows = Vec::new();
        let mut i = 0;
        while i < items.len() {
            match &items[i] {
                Item::Folder(folder) => {
                    rows.push(self.render_folder(space, folder, slot(i), window, cx).into_any_element());
                    i += 1;
                }
                Item::Tab(id) => {
                    let id = *id;
                    let partner = self.ws.split_of(id).and_then(|g| g.partner(id));
                    match items.get(i + 1) {
                        Some(Item::Tab(next)) if partner == Some(*next) => {
                            let first = self.render_tab_row(id, slot(i), space, window, cx);
                            let second = self.render_tab_row(*next, slot(i + 1), space, window, cx);
                            rows.push(self.render_split_pair(id, first, second, cx));
                            i += 2;
                        }
                        _ => {
                            rows.push(self.render_tab_row(id, slot(i), space, window, cx));
                            i += 1;
                        }
                    }
                }
            }
        }
        rows
    }

    fn render_folder(
        &mut self,
        space: SpaceId,
        folder: &Folder,
        slot: Slot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = cx.tokens().clone();
        let fid = folder.id;
        let expanded = folder.is_expanded;
        let contains_selected = self.selected().is_some_and(|s| folder.tabs().contains(&s));
        let renaming = self
            .rename
            .as_ref()
            .filter(|(r, _)| *r == RenameTarget::Folder(fid))
            .map(|(_, i)| i.clone());
        // The stock folder gets Zen's filled two-tone glyph; a symbol the
        // user picked stays a Lucide outline.
        let glyph: AnyElement = if folder.symbol == "folder" {
            svg()
                .path(if expanded { "mori/folder-open-filled.svg" } else { "mori/folder-filled.svg" })
                .size(px(20.))
                .flex_none()
                .text_color(t.primary)
                .into_any_element()
        } else {
            icon(&folder.symbol, px(18.), t.primary).into_any_element()
        };
        let name: SharedString = folder.name.clone().into();
        // While a folder is dragged, the top of each header takes drops
        // "before" it; the rest of the header puts things inside.
        let folder_drag = cx.has_active_drag() && cx.active_drag_cursor_style() != Some(CursorStyle::ResizeColumn);
        let before_zone = folder_drag.then(|| {
            div()
                .id(SharedString::from(format!("folder-before-{fid}")))
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .h(px(10.))
                .drag_over::<DraggedFolder>(move |s, _, _, _| s.border_t_2().border_color(t.primary.opacity(0.8)))
                .drag_over::<DraggedTab>(move |s, _, _, _| s.border_t_2().border_color(t.primary.opacity(0.8)))
                .on_drop(cx.listener(move |this, d: &DraggedFolder, window, cx| {
                    if d.id != fid {
                        this.drop_folder(d, DropTarget::Slot(space, slot), window, cx)
                    }
                }))
                .on_drop(cx.listener(move |this, d: &DraggedTab, window, cx| {
                    this.drop_tab(d, DropTarget::Slot(space, slot), window, cx)
                }))
        });
        let header = h_flex()
            .id(SharedString::from(format!("folder-{fid}")))
            .relative()
            .h(metrics::ROW_HEIGHT)
            .pl(px(5.))
            .pr(px(6.))
            .gap(px(8.))
            .rounded(metrics::ROW_RADIUS)
            .hover(|s| s.bg(t.tab_hover()))
            .cursor_pointer()
            .child(div().size(px(20.)).flex_none().flex().items_center().justify_center().child(glyph))
            .child(match renaming {
                Some(input) => div()
                    .flex_1()
                    .child(rename_input(&input, crate::theme::ui_font().size, FontWeight::SEMIBOLD, cx))
                    .into_any_element(),
                None => div()
                    .flex_1()
                    .truncate()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(name.clone())
                    .into_any_element(),
            })
            .when(!expanded && contains_selected, |s| {
                s.child(div().size(px(5.)).rounded_full().bg(t.primary))
            })
            .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                // A double-click renames. Its first click already toggled
                // the folder, so the second one toggles it back.
                this.ws.update_folder(fid, |f| f.is_expanded = !f.is_expanded);
                if ev.click_count() == 2 {
                    this.begin_rename(RenameTarget::Folder(fid), window, cx);
                }
                this.schedule_save(cx);
                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.menu = Some(OpenMenu {
                        kind: MenuKind::Folder(fid),
                        position: ev.position,
                    });
                    cx.notify();
                }),
            )
            .on_drag(
                DraggedFolder {
                    id: fid,
                    name,
                    origin: (space, slot),
                },
                |d, _, _, cx| cx.new(|_| d.clone()),
            )
            .drag_over::<DraggedTab>(move |s, _, _, _| {
                s.bg(t.primary.opacity(0.14)).border_1().border_color(t.primary.opacity(0.7))
            })
            .drag_over::<DraggedFolder>(move |s, d, _, _| {
                if d.id == fid {
                    s
                } else {
                    s.bg(t.primary.opacity(0.14)).border_1().border_color(t.primary.opacity(0.7))
                }
            })
            .on_drop(cx.listener(move |this, d: &DraggedTab, window, cx| {
                this.drop_tab(d, DropTarget::FolderEnd(space, fid), window, cx)
            }))
            .on_drop(cx.listener(move |this, d: &DraggedFolder, window, cx| {
                if d.id != fid {
                    this.drop_folder(d, DropTarget::FolderEnd(space, fid), window, cx)
                }
            }))
            .children(before_zone);
        let mut col = v_flex().gap(metrics::ROW_GAP).child(header);
        if expanded && !folder.items.is_empty() {
            // Children are indented behind a guide line under the folder.
            let rows = self.render_items(space, &folder.items, Some(fid), window, cx);
            let children = v_flex()
                .relative()
                .pl(px(12.))
                .gap(metrics::ROW_GAP)
                .child(
                    div()
                        .absolute()
                        .left(px(14.))
                        .top(px(2.))
                        .bottom(px(2.))
                        .w(px(1.))
                        .bg(t.sidebar_separator()),
                )
                .children(rows);
            col = col.child(children);
        }
        col
    }

    /// Two rows of a split pair, side by side in one frame.
    fn render_split_pair(&self, id: TabId, first: AnyElement, second: AnyElement, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.tokens().clone();
        h_flex()
            .id(SharedString::from(format!("split-pair-{id}")))
            .gap(px(2.))
            .p(px(2.))
            .rounded(metrics::RADIUS_LG)
            .border_1()
            .border_color(t.sidebar_foreground.opacity(0.12))
            .child(div().flex_1().min_w_0().child(first))
            .child(div().flex_1().min_w_0().child(second))
            .into_any_element()
    }

    /// Zen's divider between the pinned rows and today's tabs. While the
    /// pointer is over the sidebar its end turns into "Clear", which
    /// archives every tab below it. Dropping a tab or folder on it pins it
    /// at the end of the pinned rows.
    fn render_divider(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.tokens().clone();
        let space = self.ws.active_space;
        let clearable = !self.ws.active().loose.is_empty();
        let has_pinned = !self.ws.active().pinned.is_empty();
        let line = t.sidebar_separator();
        let muted = t.sidebar_foreground.opacity(0.5);
        let clear = h_flex()
            .id("clear-today")
            .relative()
            .flex_none()
            .h(px(20.))
            .pl(px(8.))
            .gap(px(3.))
            .items_center()
            .text_size(metrics::TEXT_CAPTION)
            .font_weight(FontWeight::MEDIUM)
            .text_color(muted)
            .cursor_pointer()
            .hover(|s| s.text_color(t.sidebar_foreground))
            // At rest this end is just more of the line.
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .group_hover("sidebar", |s| s.invisible())
                    .child(div().w_full().h(px(1.)).bg(line)),
            )
            .child(icon("arrow-down", px(10.), muted).invisible().group_hover("sidebar", |s| s.visible()))
            .child(div().invisible().group_hover("sidebar", |s| s.visible()).child("Clear"))
            .tooltip(|window, cx| gpui_kit::component::tooltip::Tooltip::new("Archive all unpinned tabs").build(window, cx))
            .on_click(cx.listener(|this, _, window, cx| this.clear_today(window, cx)));
        h_flex()
            .id("pinned-divider")
            .flex_none()
            // As in Zen: 37px from the label's cap-top to the divider, or
            // 13px under the last pinned row.
            .mt(px(if has_pinned { 3. } else { 9. }))
            .mb(px(3.))
            .h(px(20.))
            .rounded(metrics::ROW_RADIUS)
            .child(div().flex_1().h(px(1.)).ml(px(4.)).bg(line))
            .when(clearable, |s| s.child(clear))
            .drag_over::<DraggedTab>(move |s, _, _, _| {
                s.bg(t.primary.opacity(0.14)).border_1().border_dashed().border_color(t.primary.opacity(0.7))
            })
            .drag_over::<DraggedFolder>(move |s, _, _, _| {
                s.bg(t.primary.opacity(0.14)).border_1().border_dashed().border_color(t.primary.opacity(0.7))
            })
            .on_drop(cx.listener(move |this, d: &DraggedTab, window, cx| {
                this.drop_tab(d, DropTarget::PinnedEnd(space), window, cx)
            }))
            .on_drop(cx.listener(move |this, d: &DraggedFolder, window, cx| {
                this.drop_folder(d, DropTarget::PinnedEnd(space), window, cx)
            }))
    }

    fn render_loose(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.tokens().clone();
        let space = self.ws.active().clone();
        let space_id = space.id;
        // While Ctrl+T's bar is open the row reads as the tab being created.
        let creating = self.creating_tab(cx);
        let new_tab_fg = if creating { t.sidebar_foreground } else { t.muted_foreground };
        let new_tab = h_flex()
            .id("new-tab-row")
            .h(metrics::ROW_HEIGHT)
            .pl(px(7.))
            .gap(px(10.))
            .rounded(metrics::ROW_RADIUS)
            .text_color(new_tab_fg)
            .when(creating, |s| s.bg(t.tab_selected()))
            .when(!creating, |s| s.hover(|s| s.bg(t.tab_hover())))
            .cursor_pointer()
            .child(
                div()
                    .size(metrics::FAVICON)
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon("plus", px(13.), new_tab_fg)),
            )
            .child("New Tab")
            .on_click(cx.listener(|this, _, window, cx| this.open_launcher(LauncherMode::NewTab, window, cx)))
            .drag_over::<DraggedTab>(move |s, _, _, _| s.bg(t.primary.opacity(0.14)))
            .on_drop(cx.listener(move |this, d: &DraggedTab, window, cx| {
                this.drop_tab(d, DropTarget::Slot(space_id, Slot::Loose(0)), window, cx)
            }));
        let mut rows = Vec::new();
        let mut i = 0;
        while i < space.loose.len() {
            let id = space.loose[i];
            let partner = self.ws.split_of(id).and_then(|g| g.partner(id));
            if partner.is_some() && space.loose.get(i + 1).copied() == partner {
                let first = self.render_tab_row(id, Slot::Loose(i), space_id, window, cx);
                let second = self.render_tab_row(space.loose[i + 1], Slot::Loose(i + 1), space_id, window, cx);
                rows.push(self.render_split_pair(id, first, second, cx));
                i += 2;
            } else {
                rows.push(self.render_tab_row(id, Slot::Loose(i), space_id, window, cx));
                i += 1;
            }
        }
        v_flex()
            .gap(metrics::ROW_GAP)
            .pb(px(10.))
            .child(new_tab)
            .children(rows)
            .child(
                div()
                    .id("loose-end")
                    .h(px(24.))
                    .rounded(metrics::ROW_RADIUS)
                    .drag_over::<DraggedTab>(move |s, _, _, _| {
                        s.bg(t.primary.opacity(0.14))
                            .border_1()
                            .border_dashed()
                            .border_color(t.primary.opacity(0.7))
                    })
                    .on_drop(cx.listener(move |this, d: &DraggedTab, window, cx| {
                        this.drop_tab(d, DropTarget::LooseEnd(space_id), window, cx)
                    })),
            )
    }

    fn render_tab_row(
        &mut self,
        id: TabId,
        slot: Slot,
        space: SpaceId,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = cx.tokens().clone();
        let Some(record) = self.ws.tab(id).cloned() else {
            return div().into_any_element();
        };
        let fav = self.favicon_for(id);
        // Ctrl+T moves the selection to the "New Tab" row, as in Zen.
        let selected = self.active_tab() == Some(id) && !self.creating_tab(cx);
        let multi = self.multi_select.contains(&id);
        let in_split = self.visible_split_member(id);
        let live = self.live.get(&id);
        let loaded = live.is_some();
        let loading = live.is_some_and(|l| l.loading);
        let muted = live.is_some_and(|l| l.muted);
        let audible = self.is_tab_audible(id);
        let saved = slot.is_saved();
        // A pinned tab that navigated away from its pinned page (Arc's
        // slash, Zen's "Back to pinned url").
        let away = saved && record.left_pinned_url();
        let title: SharedString = record.display_title().to_string().into();
        let renaming = self
            .rename
            .as_ref()
            .filter(|(r, _)| *r == RenameTarget::Tab(id))
            .map(|(_, i)| i.clone());
        // Asleep tabs keep full colour (as in Zen and Arc); only the selected
        // row is brighter.
        let fg = if selected {
            t.sidebar_foreground
        } else {
            t.sidebar_foreground.opacity(0.9)
        };
        let group = SharedString::from(format!("tab-{id}"));
        let row_bg = if selected {
            Some(t.tab_selected())
        } else if in_split {
            Some(t.tab_selected().opacity(0.55))
        } else {
            None
        };
        // Long titles fade into the row instead of ending in an ellipsis.
        // The fade needs the row's exact colour, which a gradient wash
        // doesn't have; those themes keep the ellipsis.
        // A selected row's shadow is painted under its translucent fill and
        // shows through it, so it is part of the colour the fade must match.
        let mut layers: Vec<Hsla> = Vec::new();
        if selected {
            layers.extend(t.tab_selected_shadow().iter().map(|s| s.color));
        }
        layers.extend(row_bg);
        // A multi-selected row's tint replaces its hover fill.
        let multi_tint = t.primary.opacity(0.16);
        if multi && !selected {
            layers.push(multi_tint);
        }
        let fade = t.opaque_on_sidebar_layers(&layers).map(|base| {
            let hover = (!selected && !multi).then(|| t.opaque_on_sidebar(t.tab_hover())).flatten();
            (base, hover)
        });
        let fade_out = fade_to;
        let renaming_row = renaming.is_some();
        let title_text = h_flex()
            .min_w_0()
            .when(away, |s| {
                s.child(div().flex_none().mr(px(5.)).text_color(fg.opacity(0.45)).child("/"))
            })
            .child(title.clone());
        let title_el: AnyElement = match renaming {
            Some(input) => div()
                .flex_1()
                .child(rename_input(&input, crate::theme::ui_font().size, FontWeight::NORMAL, cx))
                .into_any_element(),
            None => match fade {
                Some((base, hover)) => div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(title_text)
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .right_0()
                            .w(px(24.))
                            .bg(fade_out(base))
                            .when_some(hover, |s, hover| {
                                s.group_hover(group.clone(), move |s| s.bg(fade_out(hover)))
                            }),
                    )
                    .into_any_element(),
                None => div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .when(away, |s| s.child(div().flex_none().text_color(fg.opacity(0.45)).child("/ ")))
                    .child(title.clone())
                    .into_any_element(),
            },
        };
        let row_button = |id: &'static str| {
            div()
                .id(id)
                .size(px(24.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .hover(|s| s.bg(if t.dark { white().opacity(0.10) } else { black().opacity(0.07) }))
        };

        // Speaker while the tab plays sound (click to mute), crossed out
        // while muted (click to unmute).
        let sound = muted || audible;
        let mute_button = |button_id: &'static str, cx: &mut Context<Self>| {
            row_button(button_id)
                .child(icon(if muted { "volume-x" } else { "volume-2" }, px(12.), fg.opacity(0.7)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.toggle_mute(id, cx);
                }))
        };
        // A loaded pinned tab offers Zen's "unload" (−) instead of close;
        // an unloaded one closes for good (with Undo).
        let unloads = saved && loaded;
        let close_button = |cx: &mut Context<Self>| {
            row_button("close")
                .child(icon(if unloads { "minus" } else { "x" }, px(12.), fg.opacity(0.7)))
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.close_tab(id, !unloads, window, cx);
                }))
        };
        // The row's colour while hovered, when it is a single opaque colour.
        let hover_backing = fade
            .filter(|_| !renaming_row)
            .map(|(base, hover)| hover.unwrap_or(base));
        // The favicon of a tab that left its pinned page turns into a
        // "back" button on hover.
        let favicon_slot = div()
            .id("tab-favicon")
            .group(SharedString::from(format!("tab-icon-{id}")))
            .relative()
            .flex_none()
            .child(
                div()
                    .when(away, |s| {
                        s.group_hover(SharedString::from(format!("tab-icon-{id}")), |s| s.invisible())
                    })
                    .child(favicon(fav.clone(), metrics::FAVICON, false, cx)),
            )
            .when(loading, |s| {
                s.child(
                    div()
                        .absolute()
                        .bottom(px(-2.))
                        .right(px(-2.))
                        .size(px(5.))
                        .rounded_full()
                        .bg(t.status_info),
                )
            })
            .when(away, |s| {
                s.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .invisible()
                        .group_hover(SharedString::from(format!("tab-icon-{id}")), |s| s.visible())
                        .child(icon("undo-2", px(14.), fg)),
                )
                .cursor_pointer()
                .tooltip(|window, cx| gpui_kit::component::tooltip::Tooltip::new("Back to pinned page").build(window, cx))
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.reset_pinned_tab(id, window, cx);
                }))
            });
        let drag_ids = self.drag_group(id);

        h_flex()
            .id(SharedString::from(format!("tab-row-{id}")))
            .group(group.clone())
            .relative()
            .h(metrics::ROW_HEIGHT)
            .pl(px(7.))
            .pr(px(6.))
            .gap(px(10.))
            .rounded(metrics::ROW_RADIUS)
            .text_color(fg)
            .when_some(row_bg, |s, bg| s.bg(bg))
            .when(selected, |s| s.shadow(t.tab_selected_shadow()))
            .when(!selected && !multi, |s| s.hover(|s| s.bg(t.tab_hover())))
            .when(multi && !selected, |s| s.bg(multi_tint))
            .when(multi, |s| s.border_1().border_color(t.primary.opacity(0.6)))
            .cursor_pointer()
            .child(favicon_slot)
            .child(title_el)
            .when(sound, |s| s.child(mute_button("mute", cx)))
            // Close appears on hover only, for the selected row too (Zen).
            // Hover may only toggle visibility, never `display`: GPUI panics
            // when an element's display differs between prepaint and paint,
            // which happens whenever rows shift under a resting pointer.
            .map(|row| match hover_backing {
                // The title keeps the whole row until hover; the buttons then
                // cover its end on the row's own (hover) colour.
                Some(backing) => row.child(
                    h_flex()
                        .absolute()
                        .top(px(2.))
                        .bottom(px(2.))
                        .right(px(6.))
                        .invisible()
                        .group_hover(group.clone(), |s| s.visible())
                        .child(div().w(px(24.)).h_full().bg(fade_to(backing)))
                        .child(
                            h_flex()
                                .h_full()
                                .items_center()
                                .bg(backing)
                                .when(sound, |s| s.child(mute_button("mute-hover", cx)))
                                .child(close_button(cx)),
                        ),
                ),
                // Under a gradient wash (or while renaming) there is no single
                // colour to cover the title with, so the button keeps its slot.
                None => row.child(close_button(cx).invisible().group_hover(group.clone(), |s| s.visible())),
            })
            .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| this.click_tab(id, ev, window, cx)))
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(move |this, _, window, cx| this.close_tab(id, !saved, window, cx)),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.menu = Some(OpenMenu {
                        kind: MenuKind::Tab(id),
                        position: ev.position,
                    });
                    cx.notify();
                }),
            )
            .on_drag(
                DraggedTab {
                    id,
                    ids: drag_ids,
                    title,
                    favicon: fav,
                    origin: Some((space, slot)),
                },
                |d, _, _, cx| cx.new(|_| d.clone()),
            )
            .drag_over::<DraggedTab>(move |s, d, _, _| {
                let s = s.border_color(t.primary.opacity(0.8));
                if d.lands_after(space, slot) {
                    s.border_b_2()
                } else {
                    s.border_t_2()
                }
            })
            .on_drop(cx.listener(move |this, d: &DraggedTab, window, cx| {
                this.drop_tab(d, DropTarget::Onto(space, slot), window, cx)
            }))
            // Folders reorder among the pinned rows (they never go below
            // the divider).
            .when(slot.is_pinned(), |row| {
                row.drag_over::<DraggedFolder>(move |s, d, _, _| {
                    let s = s.border_color(t.primary.opacity(0.8));
                    if lands_after(Some(d.origin), space, slot) {
                        s.border_b_2()
                    } else {
                        s.border_t_2()
                    }
                })
                .on_drop(cx.listener(move |this, d: &DraggedFolder, window, cx| {
                    this.drop_folder(d, DropTarget::Onto(space, slot), window, cx)
                }))
            })
            .into_any_element()
    }

    /// The bottom bar; `bar_width` is the width its buttons and chips share.
    fn render_bottom_bar(&mut self, bar_width: f32, cx: &mut Context<Self>) -> Div {
        let t = cx.tokens().clone();
        let active = self.ws.active_space;
        let ai = self.settings.ai_integration_enabled;
        let ai_open = self.assistant_visible;
        let menu_closed = self.menu.is_none();
        let button = metrics::TOOLBAR_BUTTON;
        // Zen's switcher: a dot per space (an icon when the user picked one),
        // the active one brighter; hidden while there is only one space.
        let (rest, current) = if t.dark {
            (white().opacity(0.27), white().opacity(0.62))
        } else {
            (black().opacity(0.25), black().opacity(0.6))
        };
        let hover = if t.dark { white().opacity(0.45) } else { black().opacity(0.42) };
        // Zen spaces its dots 32px apart; with more spaces they close up to
        // fit between the buttons before the row starts to scroll.
        let buttons = if ai { 3. } else { 2. };
        let room = bar_width - buttons * f32::from(button);
        let count = self.ws.spaces.len().max(1) as f32;
        let pitch = (room / count).clamp(16., 32.);
        let chip = px(pitch.min(24.));
        let gap = px((pitch - 24.).max(0.));
        // Centred while they fit; once they overflow they start at the left
        // edge so the row can scroll to every chip (centring would push the
        // first ones out of reach).
        let overflow = pitch * count > room + 0.5;
        let chips: Vec<AnyElement> = if self.ws.spaces.len() < 2 {
            Vec::new()
        } else {
            self.ws
                .spaces
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    let id = s.id;
                    let is_active = id == active;
                    let color = if is_active { current } else { rest };
                    let glyph: AnyElement = if s.symbol == DEFAULT_SPACE_SYMBOL {
                        div()
                            .size(px(8.))
                            .rounded_full()
                            .bg(color)
                            .when(!is_active, |d| d.group_hover("space-chip", |d| d.bg(hover)))
                            .into_any_element()
                    } else {
                        space_glyph(&s.symbol, px(14.), color)
                    };
                    div()
                        .id(("space-chip", i))
                        .group("space-chip")
                        .relative()
                        .child(crate::ui::menus::anchor_probe(crate::ui::menus::space_chip_anchor(id)))
                        .size(chip)
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .rounded(px(6.))
                        .hover(|s| s.bg(t.tab_hover()))
                        .cursor_pointer()
                        .child(glyph)
                        // A space's menu opens over its chip; its tooltip mustn't cover it.
                        .when(menu_closed, |chip| {
                            chip.tooltip({
                                let name: SharedString = s.name.clone().into();
                                move |window, cx| gpui_kit::component::tooltip::Tooltip::new(name.clone()).build(window, cx)
                            })
                        })
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if this.ws.active_space == id {
                                this.open_space_editor(id, None, window, cx);
                            } else {
                                this.switch_space(id, window, cx);
                            }
                        }))
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                                cx.stop_propagation();
                                this.menu = Some(OpenMenu {
                                    kind: MenuKind::Space(id),
                                    position: ev.position,
                                });
                                cx.notify();
                            }),
                        )
                        .on_drag(
                            DraggedSpace {
                                id,
                                symbol: s.symbol.clone().into(),
                                index: i,
                            },
                            |d, _, _, cx| cx.new(|_| d.clone()),
                        )
                        .drag_over::<DraggedTab>(move |s, _, _, _| s.bg(t.primary.opacity(0.2)))
                        .drag_over::<DraggedFolder>(move |s, _, _, _| s.bg(t.primary.opacity(0.2)))
                        .drag_over::<DraggedSpace>(move |s, d, _, _| {
                            let s = s.border_color(t.primary.opacity(0.8));
                            match d.index.cmp(&i) {
                                std::cmp::Ordering::Less => s.border_r_2(),
                                std::cmp::Ordering::Greater => s.border_l_2(),
                                std::cmp::Ordering::Equal => s,
                            }
                        })
                        .on_drop(cx.listener(move |this, d: &DraggedTab, window, cx| {
                            this.drop_tab(d, DropTarget::Space(id), window, cx)
                        }))
                        .on_drop(cx.listener(move |this, d: &DraggedFolder, window, cx| {
                            this.drop_folder(d, DropTarget::Space(id), window, cx)
                        }))
                        .on_drop(cx.listener(move |this, d: &DraggedSpace, _, cx| {
                            // Onto a chip to the right: after it; to the left: before it.
                            let index = if d.index < i { i + 1 } else { i };
                            this.ws.move_space(d.id, index);
                            this.schedule_save(cx);
                            cx.notify();
                        }))
                        .into_any_element()
                })
                .collect()
        };

        h_flex()
            .h(px(40.))
            .flex_none()
            .ml(px(-4.))
            .mr(px(-3.))
            .child(
                icon_button("library", "library-big", button, true, cx).flex_none().on_click(cx.listener(
                    |this, _, _, cx| {
                        this.panel = (this.panel != Some(Panel::Library)).then_some(Panel::Library);
                        cx.notify();
                    },
                )),
            )
            .when(ai, |s| {
                s.child(
                    icon_button("assistant", "sparkles", button, true, cx)
                        .flex_none()
                        .when(ai_open, |b| b.bg(t.primary.opacity(0.2)))
                        .on_click(cx.listener(|this, _, window, cx| this.toggle_assistant(window, cx))),
                )
            })
            .child(
                h_flex()
                    .id("space-switcher")
                    .flex_1()
                    .min_w_0()
                    .when(!overflow, |s| s.justify_center())
                    .gap(gap)
                    .overflow_x_scroll()
                    .children(chips),
            )
            .child(
                div()
                    .id("new-menu")
                    .size(button)
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .rounded(metrics::RADIUS)
                    .relative()
                    .child(crate::ui::menus::anchor_probe(crate::ui::menus::NEW_MENU_ANCHOR))
                    .cursor_pointer()
                    .hover(|s| s.bg(t.ghost_hover()))
                    .active(|s| s.bg(t.ghost_pressed()))
                    // Zen turns its "+" into an "x" while the menu is open.
                    .child(icon(
                        if self.menu.as_ref().is_some_and(|m| matches!(m.kind, MenuKind::NewMenu)) { "x" } else { "plus" },
                        px(14.),
                        t.sidebar_foreground.opacity(if t.dark { 0.65 } else { 0.6 }),
                    ))
                    .on_click(cx.listener(|this, ev: &ClickEvent, _, cx| {
                        this.menu = Some(OpenMenu {
                            kind: MenuKind::NewMenu,
                            position: ev.position(),
                        });
                        cx.notify();
                    })),
            )
    }
}

/// A space's symbol: an emoji drawn as text, or a Lucide icon.
pub fn space_glyph(symbol: &str, size: Pixels, color: Hsla) -> AnyElement {
    if mori_core::workspace::is_emoji_symbol(symbol) {
        div()
            .flex_none()
            .text_size(size)
            .line_height(size + px(2.))
            .child(SharedString::from(symbol.to_string()))
            .into_any_element()
    } else {
        icon(symbol, size, color).into_any_element()
    }
}

/// A horizontal fade from transparent into `color`, laid over the end of
/// text that runs under something drawn on `color`.
fn fade_to(color: Hsla) -> Background {
    linear_gradient(
        90.,
        linear_color_stop(color.opacity(0.), 0.),
        linear_color_stop(color, 1.),
    )
}

/// A lettered tile for a site without a favicon, coloured from its host so
/// pins stay tellable apart.
fn monogram(host: &str, cx: &App) -> AnyElement {
    let Some(letter) = mori_core::theme::monogram_letter(host) else {
        return favicon(None, metrics::FAVICON, false, cx);
    };
    div()
        .size(metrics::FAVICON)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .bg(crate::theme::hsla(mori_core::theme::monogram_color(host)))
        .text_size(px(10.))
        .font_weight(FontWeight::BOLD)
        .text_color(white())
        .child(letter.to_string())
        .into_any_element()
}

/// Empty header space that moves the window (client-side decorations);
/// a double click maximizes.
fn window_drag_area(id: &'static str) -> Stateful<Div> {
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

/// An inline rename field that sits exactly where its label was: no padding,
/// the label's size and weight, and a thin accent underline while editing.
fn rename_input(
    input: &Entity<gpui_kit::component::input::InputState>,
    size: Pixels,
    weight: FontWeight,
    cx: &App,
) -> impl IntoElement {
    let t = cx.tokens();
    let field = Input::new(input)
        .appearance(false)
        .bordered(false)
        .focus_bordered(false)
        .px_0()
        .py_0()
        .text_size(size)
        .font_weight(weight);
    // `Input::h` only sizes multi-line inputs; the style height applies here.
    div().border_b_1().border_color(t.primary.opacity(0.6)).child(Styled::h(field, size + px(6.)))
}

/// Minimize / maximize / close buttons, shown only when Mori draws its own
/// decorations and the window isn't tiled by the compositor.
fn window_controls(window: &Window, cx: &App) -> Option<Div> {
    let Decorations::Client { tiling } = window.window_decorations() else {
        return None;
    };
    if tiling.top || tiling.left || tiling.right || tiling.bottom {
        return None;
    }
    Some(
        h_flex()
            .ml_1()
            .gap_0p5()
            .child(
                icon_button("win-min", "minus", px(24.), true, cx)
                    .on_click(|_, window, _| window.minimize_window()),
            )
            .child(
                icon_button("win-max", "square", px(24.), true, cx)
                    .on_click(|_, window, _| window.zoom_window()),
            )
            .child(
                icon_button("win-close", "x", px(24.), true, cx)
                    .on_click(|_, window, _| window.remove_window()),
            ),
    )
}
