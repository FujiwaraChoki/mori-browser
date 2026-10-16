//! The sidebar: navigation, address pill, pinned grid, folders, tabs, and
//! the space switcher.

use std::sync::Arc;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{component::input::Input, *};
use mori_core::{
    omnibox,
    util::host_of,
    workspace::{Folder, Slot, SpaceId, TabId},
};

use crate::{
    browser::{Browser, MenuKind, OpenMenu, Panel, RenameTarget},
    theme::{ActiveTokens, metrics},
    ui::{
        components::{favicon, h_flex, icon, icon_button, v_flex},
        launcher::LauncherMode,
    },
};

/// Payload while a tab is being dragged.
#[derive(Clone)]
pub struct DraggedTab {
    pub id: TabId,
    pub title: SharedString,
    pub favicon: Option<Arc<RenderImage>>,
}

impl Render for DraggedTab {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens().clone();
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
            .child(favicon(self.favicon.clone(), metrics::FAVICON, false, cx))
            .child(div().truncate().child(self.title.clone()))
    }
}

/// Where a dropped tab lands.
#[derive(Clone, Copy, Debug)]
pub enum DropTarget {
    Slot(SpaceId, Slot),
    FolderEnd(SpaceId, mori_core::workspace::FolderId),
    LooseEnd(SpaceId),
    PinnedEnd(SpaceId),
    Space(SpaceId),
}

/// The symbol new spaces start with; the switcher draws it as a plain dot.
pub const DEFAULT_SPACE_SYMBOL: &str = "circle";

/// Space glyphs offered in the space editor (Lucide icon names).
pub const SPACE_SYMBOLS: [&str; 24] = [
    "circle", "house", "briefcase", "code", "book-open", "music", "gamepad-2", "palette",
    "coffee", "leaf", "heart", "star", "flame", "zap", "globe", "graduation-cap", "camera",
    "plane", "shopping-bag", "dumbbell", "sparkles", "moon", "sun", "folder",
];

impl Browser {
    fn favicon_for(&mut self, id: TabId) -> Option<Arc<RenderImage>> {
        let host = self.ws.tab(id).and_then(|t| host_of(&t.url))?;
        self.favicons.get(&host)
    }

    pub fn drop_tab(&mut self, tab: TabId, target: DropTarget, window: &mut Window, cx: &mut Context<Self>) {
        match target {
            DropTarget::Slot(space, slot) => {
                self.ws.move_tab(tab, space, slot);
            }
            DropTarget::FolderEnd(space, folder) => {
                let end = self
                    .ws
                    .space(space)
                    .and_then(|s| s.folder(folder))
                    .map(|f| f.tab_ids.len())
                    .unwrap_or(0);
                self.ws.move_tab(tab, space, Slot::Folder(folder, end));
            }
            DropTarget::LooseEnd(space) => {
                let end = self.ws.space(space).map(|s| s.loose.len()).unwrap_or(0);
                self.ws.move_tab(tab, space, Slot::Loose(end));
            }
            DropTarget::PinnedEnd(space) => {
                let end = self.ws.space(space).map(|s| s.pinned.len()).unwrap_or(0);
                self.ws.move_tab(tab, space, Slot::Pinned(end));
            }
            DropTarget::Space(space) => {
                self.move_tab_to_space(tab, space, window, cx);
            }
        }
        self.schedule_save(cx);
        cx.notify();
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

        v_flex()
            .id("sidebar")
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
            .child(self.render_header(left, window, cx).pl(pl).pr(pr))
            .child(div().flex_none().pl(pl).pr(pr).child(self.render_pinned(cx)))
            .child(
                v_flex()
                    .id("sidebar-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .pl(pl + metrics::ROW_INSET)
                    .pr(pr + metrics::ROW_INSET)
                    .child(self.render_space_header(cx))
                    .child(self.render_folders(window, cx))
                    // Zen's divider between the pinned section and the tabs.
                    .child(div().flex_none().mt(px(10.)).mb(px(12.)).h(px(1.)).bg(t.sidebar_separator()))
                    .child(self.render_loose(window, cx)),
            )
            .children(self.render_media_strip(cx))
            .child(self.render_bottom_bar(cx).pl(pl).pr(pr))
    }

    fn render_header(&mut self, left: bool, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = cx.tokens().clone();
        let live = self.selected_live();
        let (can_back, can_fwd, loading) = live
            .map(|l| (l.can_go_back, l.can_go_forward, l.loading))
            .unwrap_or((false, false, false));
        let has_downloads = !self.downloads.is_empty();
        let active_downloads = self
            .downloads
            .iter()
            .filter(|d| d.state == mori_engine::DownloadState::InProgress)
            .count();
        let button = metrics::TOOLBAR_BUTTON;

        // Zen's toolbar: app menu and sidebar toggle on the left, navigation
        // on the right; 30px buttons with 16px glyphs and no gaps.
        let nav = h_flex()
            .h(button)
            .ml(px(-4.))
            .mr(px(-3.))
            .child(
                icon_button("app-menu", "ellipsis", button, true, cx).on_click(cx.listener(
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
            .child(
                // Empty header space moves the window (client-side decorations).
                div()
                    .id("window-drag")
                    .flex_1()
                    .h_full()
                    .on_mouse_down(MouseButton::Left, |ev: &MouseDownEvent, window, _| {
                        if ev.click_count == 2 {
                            window.zoom_window();
                        } else {
                            window.start_window_move();
                        }
                    }),
            )
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
                        .when(active_downloads > 0, |s| {
                            s.child(
                                div()
                                    .absolute()
                                    .top(px(5.))
                                    .right(px(5.))
                                    .size(px(6.))
                                    .rounded_full()
                                    .bg(t.status_info),
                            )
                        }),
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

        let nav = nav.children(window_controls(window, cx));
        v_flex()
            .pt(px(6.))
            .pb(px(10.))
            .gap(px(7.))
            .child(nav)
            .child(self.render_omnibox(cx))
    }

    fn render_omnibox(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.tokens().clone();
        let font = crate::theme::ui_font();
        let record = self.selected().and_then(|id| self.ws.tab(id)).cloned();
        let url = record.as_ref().map(|r| r.url.clone()).unwrap_or_default();
        let host = omnibox::display_host(&url);
        let is_web = url.starts_with("http://") || url.starts_with("https://");
        let bookmarked = is_web && self.bookmarks.contains(&url);
        let reader = self.selected_live().is_some_and(|l| l.reader);
        let muted = t.sidebar_foreground.opacity(if t.dark { 0.6 } else { 0.55 });

        // Zen shows the bare host; only insecure pages and the empty state
        // get a leading glyph.
        let lead = if url.starts_with("http://") {
            Some(icon("triangle-alert", px(14.), t.status_warning))
        } else if host.is_empty() {
            Some(icon("search", px(14.), muted))
        } else {
            None
        };
        // Page actions appear on hover, or stay while they show state.
        let action = |id: &'static str, glyph: &str, active: bool, color: Hsla| {
            div()
                .id(id)
                .size(px(26.))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .rounded(px(7.))
                .when(!active, |s| s.hidden().group_hover("omnibox", |s| s.flex()))
                .hover(|s| s.bg(t.tab_hover()))
                .cursor_pointer()
                .child(icon(glyph, px(14.), color))
        };

        h_flex()
            .id("omnibox")
            .group("omnibox")
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
            .children(lead)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(font.url_size())
                    .font_weight(FontWeight::MEDIUM)
                    .when(host.is_empty(), |s| s.text_color(muted).child("Search..."))
                    .when(!host.is_empty(), |s| {
                        s.text_color(t.sidebar_foreground.opacity(if t.dark { 0.75 } else { 0.68 }))
                            .child(host)
                    }),
            )
            .when(is_web, |s| {
                s.child(
                    action("reader", "book-open-text", reader, if reader { t.primary } else { muted })
                        .on_click(cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            this.toggle_reader(cx);
                        })),
                )
                .child(
                    action("bookmark", "star", bookmarked, if bookmarked { t.status_warning } else { muted })
                        .on_click(cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            this.toggle_bookmark(cx);
                        })),
                )
            })
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
            .mt(px(8.))
            .pl(px(7.))
            .gap(px(6.))
            .children(custom_symbol.map(|symbol| icon(&symbol, px(13.), t.muted_foreground)))
            .child(match renaming {
                Some(input) => div()
                    .flex_1()
                    .child(Input::new(&input).appearance(false).bordered(false))
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

    fn render_pinned(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.tokens().clone();
        let space = self.ws.active().clone();
        let pins = space.pinned.clone();
        if pins.is_empty() {
            // A slim drop zone so tabs can be pinned by dragging.
            return div()
                .id("pin-zone")
                .h(px(6.))
                .rounded(metrics::RADIUS)
                .drag_over::<DraggedTab>(move |s, _, _, _| {
                    s.h(px(40.))
                        .bg(t.primary.opacity(0.14))
                        .border_1()
                        .border_dashed()
                        .border_color(t.primary.opacity(0.7))
                })
                .on_drop(cx.listener(move |this, d: &DraggedTab, window, cx| {
                    this.drop_tab(d.id, DropTarget::PinnedEnd(space.id), window, cx)
                }))
                .into_any_element();
        }
        let selected = self.selected();
        let (selected_bg, selected_ring) = if t.dark {
            (white().opacity(0.20), white().opacity(0.45))
        } else {
            (white().opacity(0.80), black().opacity(0.15))
        };
        let mut tiles: Vec<AnyElement> = pins
            .iter()
            .enumerate()
            .map(|(i, id)| {
                let id = *id;
                let fav = self.favicon_for(id);
                let is_selected = selected == Some(id);
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
                let space_id = space.id;
                div()
                    .id(("pin", i))
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
                    .on_click(cx.listener(move |this, _, window, cx| this.select_tab(id, window, cx)))
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
                            title,
                            favicon: fav,
                        },
                        |d, _, _, cx| cx.new(|_| d.clone()),
                    )
                    .drag_over::<DraggedTab>(move |s, _, _, _| s.border_color(t.primary.opacity(0.7)))
                    .on_drop(cx.listener(move |this, d: &DraggedTab, window, cx| {
                        this.drop_tab(d.id, DropTarget::Slot(space_id, Slot::Pinned(i)), window, cx)
                    }))
                    .into_any_element()
            })
            .collect();
        // Essentials-style grid: rows of up to four tiles that always fill
        // the sidebar's width, balanced so no row has holes.
        let mut rows = v_flex().gap(px(7.)).pb(px(4.));
        for len in mori_core::theme::balanced_rows(tiles.len(), 4) {
            let rest = tiles.split_off(len);
            rows = rows.child(h_flex().gap(px(7.)).children(std::mem::replace(&mut tiles, rest)));
        }
        rows.into_any_element()
    }

    fn render_folders(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let space = self.ws.active().clone();
        let mut out = v_flex().gap(metrics::ROW_GAP);
        for folder in &space.folders {
            out = out.child(self.render_folder(space.id, folder, window, cx));
        }
        out
    }

    fn render_folder(
        &mut self,
        space: SpaceId,
        folder: &Folder,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = cx.tokens().clone();
        let fid = folder.id;
        let expanded = folder.is_expanded;
        let contains_selected = self.selected().is_some_and(|s| folder.tab_ids.contains(&s));
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
        let header = h_flex()
            .id(SharedString::from(format!("folder-{fid}")))
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
                    .child(Input::new(&input).appearance(false).bordered(false))
                    .into_any_element(),
                None => div()
                    .flex_1()
                    .truncate()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(folder.name.clone())
                    .into_any_element(),
            })
            .when(!expanded && contains_selected, |s| {
                s.child(div().size(px(5.)).rounded_full().bg(t.primary))
            })
            .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                if ev.click_count() == 2 {
                    this.begin_rename(RenameTarget::Folder(fid), window, cx);
                    return;
                }
                this.ws.update_folder(fid, |f| f.is_expanded = !f.is_expanded);
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
            .drag_over::<DraggedTab>(move |s, _, _, _| {
                s.bg(t.primary.opacity(0.14)).border_1().border_color(t.primary.opacity(0.7))
            })
            .on_drop(cx.listener(move |this, d: &DraggedTab, window, cx| {
                this.drop_tab(d.id, DropTarget::FolderEnd(space, fid), window, cx)
            }));
        let mut col = v_flex().gap(metrics::ROW_GAP).child(header);
        if expanded && !folder.tab_ids.is_empty() {
            // Children are indented behind a guide line under the folder.
            let mut children = v_flex()
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
                );
            for (i, id) in folder.tab_ids.clone().into_iter().enumerate() {
                children = children.child(self.render_tab_row(id, Slot::Folder(fid, i), space, window, cx));
            }
            col = col.child(children);
        }
        col
    }

    fn render_loose(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.tokens().clone();
        let space = self.ws.active().clone();
        let space_id = space.id;
        let new_tab = h_flex()
            .id("new-tab-row")
            .h(metrics::ROW_HEIGHT)
            .pl(px(7.))
            .gap(px(10.))
            .rounded(metrics::ROW_RADIUS)
            .text_color(t.muted_foreground)
            .hover(|s| s.bg(t.tab_hover()))
            .cursor_pointer()
            .child(
                div()
                    .size(metrics::FAVICON)
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon("plus", px(13.), t.muted_foreground)),
            )
            .child("New Tab")
            .on_click(cx.listener(|this, _, window, cx| this.open_launcher(LauncherMode::NewTab, window, cx)))
            .drag_over::<DraggedTab>(move |s, _, _, _| s.bg(t.primary.opacity(0.14)))
            .on_drop(cx.listener(move |this, d: &DraggedTab, window, cx| {
                this.drop_tab(d.id, DropTarget::Slot(space_id, Slot::Loose(0)), window, cx)
            }));
        let mut col = v_flex().gap(metrics::ROW_GAP).pb(px(10.)).child(new_tab);
        for (i, id) in space.loose.iter().enumerate() {
            col = col.child(self.render_tab_row(*id, Slot::Loose(i), space_id, window, cx));
        }
        col.child(
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
                    this.drop_tab(d.id, DropTarget::LooseEnd(space_id), window, cx)
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
        let selected = self.selected() == Some(id);
        let multi = self.multi_select.contains(&id);
        let in_split = self.split == Some(id);
        let live = self.live.get(&id);
        let loading = live.is_some_and(|l| l.loading);
        let muted = live.is_some_and(|l| l.muted);
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
        let fade = t.opaque_on_sidebar(row_bg.unwrap_or(transparent_black())).map(|base| {
            let hover = (!selected).then(|| t.opaque_on_sidebar(t.tab_hover())).flatten();
            (base, hover)
        });
        let fade_out = |color: Hsla| {
            linear_gradient(
                90.,
                linear_color_stop(color.opacity(0.), 0.),
                linear_color_stop(color, 1.),
            )
        };
        let title_el: AnyElement = match renaming {
            Some(input) => div()
                .flex_1()
                .child(Input::new(&input).appearance(false).bordered(false))
                .into_any_element(),
            None => match fade {
                Some((base, hover)) => div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(title.clone())
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
                None => div().flex_1().min_w_0().truncate().child(title.clone()).into_any_element(),
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

        h_flex()
            .id(SharedString::from(format!("tab-row-{id}")))
            .group(group.clone())
            .h(metrics::ROW_HEIGHT)
            .pl(px(7.))
            .pr(px(6.))
            .gap(px(10.))
            .rounded(metrics::ROW_RADIUS)
            .text_color(fg)
            .when_some(row_bg, |s, bg| s.bg(bg))
            .when(selected, |s| s.shadow(t.tab_selected_shadow()))
            .when(!selected, |s| s.hover(|s| s.bg(t.tab_hover())))
            .when(multi, |s| s.border_1().border_color(t.primary.opacity(0.8)))
            .cursor_pointer()
            .child(
                div()
                    .relative()
                    .child(favicon(fav.clone(), metrics::FAVICON, false, cx))
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
                    }),
            )
            .child(title_el)
            .when(muted, |s| {
                s.child(
                    row_button("mute")
                        .child(icon("volume-x", px(12.), fg.opacity(0.7)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.toggle_mute(id, cx);
                        })),
                )
            })
            .child(
                // Close appears on hover only, for the selected row too (Zen).
                row_button("close")
                    .hidden()
                    .group_hover(group, |s| s.flex())
                    .child(icon("x", px(12.), fg.opacity(0.7)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.close_tab(id, true, window, cx);
                    })),
            )
            .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                let m = ev.modifiers();
                if m.secondary() {
                    if let Some(i) = this.multi_select.iter().position(|t| *t == id) {
                        this.multi_select.remove(i);
                    } else {
                        this.multi_select.push(id);
                    }
                    cx.notify();
                    return;
                }
                if m.shift
                    && let Some(anchor) = this.selected()
                {
                    let order = this.ws.active().visible();
                    let (a, b) = (
                        order.iter().position(|t| *t == anchor),
                        order.iter().position(|t| *t == id),
                    );
                    if let (Some(a), Some(b)) = (a, b) {
                        let (lo, hi) = (a.min(b), a.max(b));
                        this.multi_select = order[lo..=hi].to_vec();
                        cx.notify();
                        return;
                    }
                }
                if ev.click_count() == 2 {
                    this.begin_rename(RenameTarget::Tab(id), window, cx);
                    return;
                }
                this.select_tab(id, window, cx);
            }))
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(move |this, _, window, cx| this.close_tab(id, true, window, cx)),
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
                    title,
                    favicon: fav,
                },
                |d, _, _, cx| cx.new(|_| d.clone()),
            )
            .drag_over::<DraggedTab>(move |s, _, _, _| s.border_t_2().border_color(t.primary.opacity(0.8)))
            .on_drop(cx.listener(move |this, d: &DraggedTab, window, cx| {
                this.drop_tab(d.id, DropTarget::Slot(space, slot), window, cx)
            }))
            .into_any_element()
    }

    fn render_bottom_bar(&mut self, cx: &mut Context<Self>) -> Div {
        let t = cx.tokens().clone();
        let active = self.ws.active_space;
        let ai = self.settings.ai_integration_enabled;
        let ai_open = self.assistant_visible;
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
        let width = self.sidebar_drag.unwrap_or(self.settings.clamped_sidebar_width());
        let buttons = if ai { 3. } else { 2. };
        let room = width - buttons * f32::from(button);
        let pitch = (room / self.ws.spaces.len().max(1) as f32).clamp(20., 32.);
        let chip = px(pitch.min(24.));
        let gap = px((pitch - 24.).max(0.));
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
                        icon(&s.symbol, px(14.), color).into_any_element()
                    };
                    div()
                        .id(("space-chip", i))
                        .group("space-chip")
                        .size(chip)
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .rounded(px(6.))
                        .hover(|s| s.bg(t.tab_hover()))
                        .cursor_pointer()
                        .child(glyph)
                        .tooltip({
                            let name: SharedString = s.name.clone().into();
                            move |window, cx| gpui_kit::component::tooltip::Tooltip::new(name.clone()).build(window, cx)
                        })
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if this.ws.active_space == id {
                                this.begin_rename(RenameTarget::Space(id), window, cx);
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
                        .drag_over::<DraggedTab>(move |s, _, _, _| s.bg(t.primary.opacity(0.2)))
                        .on_drop(cx.listener(move |this, d: &DraggedTab, window, cx| {
                            this.drop_tab(d.id, DropTarget::Space(id), window, cx)
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
                icon_button("library", "library-big", button, true, cx).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.panel = (this.panel != Some(Panel::Library)).then_some(Panel::Library);
                        cx.notify();
                    },
                )),
            )
            .when(ai, |s| {
                s.child(
                    icon_button("assistant", "sparkles", button, true, cx)
                        .when(ai_open, |b| b.bg(t.primary.opacity(0.2)))
                        .on_click(cx.listener(|this, _, window, cx| this.toggle_assistant(window, cx))),
                )
            })
            .child(
                h_flex()
                    .id("space-switcher")
                    .flex_1()
                    .min_w_0()
                    .justify_center()
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
                    .cursor_pointer()
                    .hover(|s| s.bg(t.ghost_hover()))
                    .active(|s| s.bg(t.ghost_pressed()))
                    .child(icon("plus", px(14.), t.sidebar_foreground.opacity(if t.dark { 0.65 } else { 0.6 })))
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
