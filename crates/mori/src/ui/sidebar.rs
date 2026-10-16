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
            .h(px(30.))
            .px_2()
            .gap_2()
            .max_w(px(220.))
            .rounded(metrics::RADIUS)
            .bg(t.popover)
            .border_1()
            .border_color(t.border.opacity(0.6))
            .shadow(t.popover_shadow())
            .text_size(metrics::TEXT_BASE)
            .text_color(t.foreground)
            .child(favicon(self.favicon.clone(), px(15.), false, cx))
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
        let width = px(self.sidebar_drag.unwrap_or(self.settings.clamped_sidebar_width()));
        let left = self.settings.sidebar_position == mori_core::settings::SidebarPosition::Left;
        let (outer, inner) = if floating { (px(8.), px(8.)) } else { (px(8.), px(0.)) };
        let (pl, pr) = if left { (outer, inner) } else { (inner, outer) };

        v_flex()
            .id("sidebar")
            .w(width)
            .h_full()
            .flex_none()
            .text_color(t.sidebar_foreground)
            .text_size(metrics::TEXT_BASE)
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
            .child(
                v_flex()
                    .id("sidebar-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .pl(pl)
                    .pr(pr)
                    .gap_1()
                    .child(self.render_space_header(cx))
                    .child(self.render_pinned(cx))
                    .child(self.render_folders(window, cx))
                    .child(self.render_loose(window, cx)),
            )
            .children(self.render_media_strip(cx))
            .child(self.render_bottom_bar(cx).pl(pl + px(2.)).pr(pr + px(2.)))
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

        let nav = h_flex()
            .h(px(28.))
            .gap_1()
            .child(
                icon_button("toggle-sidebar", if left { "panel-left" } else { "panel-right" }, px(28.), true, cx)
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
            .child(
                icon_button("back", "arrow-left", px(28.), can_back, cx).on_click(cx.listener(
                    |this, _, _, _| {
                        if let Some(v) = this.selected_view() {
                            v.go_back();
                        }
                    },
                )),
            )
            .child(
                icon_button("forward", "arrow-right", px(28.), can_fwd, cx).on_click(cx.listener(
                    |this, _, _, _| {
                        if let Some(v) = this.selected_view() {
                            v.go_forward();
                        }
                    },
                )),
            )
            .child(
                icon_button("reload", if loading { "x" } else { "rotate-cw" }, px(28.), live.is_some(), cx)
                    .on_click(cx.listener(move |this, _, _, _| {
                        if let Some(v) = this.selected_view() {
                            if loading {
                                v.stop();
                            } else {
                                v.reload(false);
                            }
                        }
                    })),
            )
            .child(
                icon_button("library", "library", px(28.), true, cx).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.panel = (this.panel != Some(Panel::Library)).then_some(Panel::Library);
                        cx.notify();
                    },
                )),
            )
            .when(has_downloads, |s| {
                s.child(
                    div()
                        .relative()
                        .child(
                            icon_button("downloads", "download", px(28.), true, cx).on_click(
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
                                    .top(px(3.))
                                    .right(px(3.))
                                    .size(px(7.))
                                    .rounded_full()
                                    .bg(t.status_info),
                            )
                        }),
                )
            });

        let nav = nav.children(window_controls(window, cx));
        v_flex()
            .pt(px(10.))
            .pb(px(6.))
            .gap_2()
            .child(nav)
            .child(self.render_omnibox(cx))
    }

    fn render_omnibox(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.tokens().clone();
        let record = self.selected().and_then(|id| self.ws.tab(id)).cloned();
        let url = record.as_ref().map(|r| r.url.clone()).unwrap_or_default();
        let host = omnibox::display_host(&url);
        let loading = self.selected_live().is_some_and(|l| l.loading);
        let is_web = url.starts_with("http://") || url.starts_with("https://");
        let bookmarked = is_web && self.bookmarks.contains(&url);
        let reader = self.selected_live().is_some_and(|l| l.reader);
        let fav = self.selected().and_then(|id| self.favicon_for(id));

        let lead: AnyElement = if url.starts_with("https://") {
            match fav {
                Some(img) => favicon(Some(img), px(15.), false, cx),
                None => icon("lock", px(13.), t.sidebar_foreground.opacity(0.6)).into_any_element(),
            }
        } else if url.starts_with("http://") {
            icon("triangle-alert", px(13.), t.status_warning).into_any_element()
        } else {
            icon("search", px(13.), t.sidebar_foreground.opacity(0.6)).into_any_element()
        };

        h_flex()
            .id("omnibox")
            .h(px(30.))
            .px(px(11.))
            .gap(px(7.))
            .rounded(metrics::RADIUS)
            .bg(t.foreground.opacity(if t.dark { 0.08 } else { 0.05 }))
            .border_1()
            .border_color(t.border.opacity(0.35))
            .cursor_text()
            .on_click(cx.listener(|this, _, window, cx| {
                this.open_launcher(LauncherMode::Address, window, cx)
            }))
            .child(div().flex_none().size(px(16.)).flex().items_center().justify_center().child(lead))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(metrics::TEXT_BASE)
                    .when(host.is_empty(), |s| {
                        s.text_color(t.muted_foreground.opacity(0.7))
                            .child("Search or enter address")
                    })
                    .when(!host.is_empty(), |s| {
                        s.text_color(t.sidebar_foreground.opacity(0.78)).child(host)
                    }),
            )
            .when(loading, |s| {
                s.child(icon("loader-circle", px(13.), t.muted_foreground).with_animation(
                    "omnibox-spin",
                    Animation::new(std::time::Duration::from_millis(900)).repeat(),
                    |svg, delta| svg.with_transformation(Transformation::rotate(percentage(delta))),
                ))
            })
            .when(is_web, |s| {
                s.child(
                    icon_button("reader", "book-open-text", px(22.), true, cx)
                        .when(reader, |b| b.bg(t.primary.opacity(0.18)))
                        .on_click(cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            this.toggle_reader(cx);
                        })),
                )
                .child(
                    div()
                        .id("bookmark")
                        .size(px(22.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(metrics::RADIUS)
                        .hover(|s| s.bg(t.ghost_hover()))
                        .cursor_pointer()
                        .child(icon(
                            "star",
                            px(13.),
                            if bookmarked { t.status_warning } else { t.sidebar_foreground.opacity(0.55) },
                        ))
                        .on_click(cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            this.toggle_bookmark(cx);
                        })),
                )
            })
    }

    fn render_space_header(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.tokens().clone();
        let space = self.ws.active().clone();
        let renaming = self
            .rename
            .as_ref()
            .filter(|(r, _)| *r == RenameTarget::Space(space.id))
            .map(|(_, i)| i.clone());
        let id = space.id;
        h_flex()
            .id("space-header")
            .h(px(22.))
            .mt_1()
            .px(px(6.))
            .gap_2()
            .child(icon(&space.symbol, px(12.), t.muted_foreground))
            .child(match renaming {
                Some(input) => div()
                    .flex_1()
                    .child(Input::new(&input).appearance(false).bordered(false))
                    .into_any_element(),
                None => div()
                    .flex_1()
                    .truncate()
                    .text_size(metrics::TEXT_LABEL)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.sidebar_foreground.opacity(0.85))
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
        let columns = if pins.len() >= 4 { 4 } else { 3 };
        let selected = self.selected();
        let tiles: Vec<AnyElement> = pins
            .iter()
            .enumerate()
            .map(|(i, id)| {
                let id = *id;
                let fav = self.favicon_for(id);
                let is_selected = selected == Some(id);
                let loading = self.live.get(&id).is_some_and(|l| l.loading);
                let asleep = !self.live.contains_key(&id);
                let title: SharedString = self
                    .ws
                    .tab(id)
                    .map(|r| r.display_title().to_string())
                    .unwrap_or_default()
                    .into();
                let space_id = space.id;
                div()
                    .id(("pin", i))
                    .relative()
                    .h(px(48.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(metrics::RADIUS)
                    .bg(if is_selected { t.tab_selected() } else { t.tile_rest() })
                    .when(is_selected, |s| s.shadow(vec![BoxShadow {
                        color: black().opacity(t.shadow_alpha(0.15, 0.05)),
                        offset: point(px(0.), px(0.8)),
                        blur_radius: px(1.5),
                        spread_radius: px(0.),
                        inset: false,
                    }]))
                    .hover(|s| s.bg(if is_selected { t.tab_selected() } else { t.tab_hover() }))
                    .cursor_pointer()
                    .child(favicon(fav.clone(), px(24.), asleep && !is_selected, cx))
                    .when(loading, |s| {
                        s.child(
                            div()
                                .absolute()
                                .bottom(px(5.))
                                .left(px(5.))
                                .size(px(6.))
                                .rounded_full()
                                .bg(t.status_info),
                        )
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
                    .drag_over::<DraggedTab>(move |s, _, _, _| s.border_1().border_color(t.primary.opacity(0.7)))
                    .on_drop(cx.listener(move |this, d: &DraggedTab, window, cx| {
                        this.drop_tab(d.id, DropTarget::Slot(space_id, Slot::Pinned(i)), window, cx)
                    }))
                    .into_any_element()
            })
            .collect();
        div()
            .grid()
            .grid_cols(columns)
            .gap(px(6.))
            .py_1()
            .children(tiles)
            .into_any_element()
    }

    fn render_folders(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.tokens().clone();
        let space = self.ws.active().clone();
        let mut out = v_flex().gap(px(2.));
        for folder in &space.folders {
            out = out.child(self.render_folder(space.id, folder, window, cx));
        }
        if !space.folders.is_empty() {
            out = out.child(div().my_1().mx_2().h(px(1.)).bg(t.sidebar_foreground.opacity(0.12)));
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
        let header = h_flex()
            .id(SharedString::from(format!("folder-{fid}")))
            .h(px(32.))
            .px(px(9.))
            .gap_2()
            .rounded(metrics::RADIUS_LG)
            .hover(|s| s.bg(t.ghost_hover()))
            .cursor_pointer()
            .child(icon(
                if expanded { "folder-open" } else { &folder.symbol },
                px(18.),
                t.primary.opacity(0.9),
            ))
            .child(match renaming {
                Some(input) => div()
                    .flex_1()
                    .child(Input::new(&input).appearance(false).bordered(false))
                    .into_any_element(),
                None => div()
                    .flex_1()
                    .truncate()
                    .font_weight(FontWeight::MEDIUM)
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
        let mut col = v_flex().gap(px(2.)).child(header);
        if expanded {
            for (i, id) in folder.tab_ids.clone().into_iter().enumerate() {
                col = col.child(
                    div()
                        .pl(px(16.))
                        .child(self.render_tab_row(id, Slot::Folder(fid, i), space, window, cx)),
                );
            }
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
            .px(px(9.))
            .gap(px(9.))
            .rounded(metrics::RADIUS)
            .text_color(t.muted_foreground)
            .hover(|s| s.bg(t.ghost_hover()))
            .cursor_pointer()
            .child(icon("plus", px(15.), t.muted_foreground))
            .child("New Tab")
            .on_click(cx.listener(|this, _, window, cx| this.open_launcher(LauncherMode::NewTab, window, cx)))
            .drag_over::<DraggedTab>(move |s, _, _, _| s.bg(t.primary.opacity(0.14)))
            .on_drop(cx.listener(move |this, d: &DraggedTab, window, cx| {
                this.drop_tab(d.id, DropTarget::Slot(space_id, Slot::Loose(0)), window, cx)
            }));
        let mut col = v_flex().gap(px(2.)).pb(px(10.)).child(new_tab);
        for (i, id) in space.loose.iter().enumerate() {
            col = col.child(self.render_tab_row(*id, Slot::Loose(i), space_id, window, cx));
        }
        col.child(
            div()
                .id("loose-end")
                .h(px(24.))
                .rounded(metrics::RADIUS)
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
        let asleep = live.is_none() && !Browser::is_internal(&record.url);
        let loading = live.is_some_and(|l| l.loading);
        let muted = live.is_some_and(|l| l.muted);
        let audible = self.is_tab_audible(id);
        let title: SharedString = record.display_title().to_string().into();
        let renaming = self
            .rename
            .as_ref()
            .filter(|(r, _)| *r == RenameTarget::Tab(id))
            .map(|(_, i)| i.clone());
        let fg = if selected {
            t.sidebar_foreground
        } else if asleep {
            t.sidebar_foreground.opacity(0.5)
        } else {
            t.sidebar_foreground.opacity(0.78)
        };
        let group = SharedString::from(format!("tab-{id}"));

        h_flex()
            .id(SharedString::from(format!("tab-row-{id}")))
            .group(group.clone())
            .h(metrics::ROW_HEIGHT)
            .pl(px(9.))
            .pr(px(6.))
            .gap(px(9.))
            .rounded(metrics::RADIUS)
            .text_color(fg)
            .when(selected, |s| {
                s.bg(t.tab_selected()).shadow(vec![BoxShadow {
                    color: black().opacity(t.shadow_alpha(0.15, 0.05)),
                    offset: point(px(0.), px(0.8)),
                    blur_radius: px(1.5),
                    spread_radius: px(0.),
                    inset: false,
                }])
            })
            .when(in_split && !selected, |s| s.bg(t.tab_selected().opacity(0.55)))
            .when(!selected, |s| s.hover(|s| s.bg(t.tab_hover())))
            .when(multi, |s| s.border_1().border_color(t.primary.opacity(0.8)))
            .cursor_pointer()
            .child(div().relative().child(favicon(fav.clone(), px(15.), asleep, cx)).when(loading, |s| {
                s.child(
                    div()
                        .absolute()
                        .bottom(px(-2.))
                        .right(px(-2.))
                        .size(px(5.))
                        .rounded_full()
                        .bg(t.status_info),
                )
            }))
            .child(match renaming {
                Some(input) => div()
                    .flex_1()
                    .child(Input::new(&input).appearance(false).bordered(false))
                    .into_any_element(),
                None => div().flex_1().min_w_0().truncate().child(title.clone()).into_any_element(),
            })
            // Speaker while the tab plays sound (click to mute), crossed out
            // while muted (click to unmute).
            .when(muted || audible, |s| {
                s.child(
                    div()
                        .id("mute")
                        .size(px(18.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(metrics::RADIUS_MD)
                        .hover(|s| s.bg(t.sidebar_foreground.opacity(0.10)))
                        .child(icon(if muted { "volume-x" } else { "volume-2" }, px(11.), fg))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.toggle_mute(id, cx);
                        })),
                )
            })
            .child(
                div()
                    .id("close")
                    .size(px(18.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(metrics::RADIUS_MD)
                    .invisible()
                    .when(selected, |s| s.visible())
                    .group_hover(group, |s| s.visible())
                    .hover(|s| s.bg(t.sidebar_foreground.opacity(0.10)))
                    .child(icon("x", px(11.), fg))
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
        let chips: Vec<AnyElement> = self
            .ws
            .spaces
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let id = s.id;
                let is_active = id == active;
                let symbol = s.symbol.clone();
                div()
                    .id(("space-chip", i))
                    .size(px(30.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(metrics::RADIUS)
                    .opacity(if is_active { 1.0 } else { 0.45 })
                    .hover(|s| s.opacity(if is_active { 1.0 } else { 0.75 }).bg(t.ghost_hover()))
                    .cursor_pointer()
                    .child(icon(&symbol, if is_active { px(15.) } else { px(13.) }, t.sidebar_foreground))
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
            .collect();

        h_flex()
            .h(px(46.))
            .flex_none()
            .gap_1()
            .when(ai, |s| {
                s.child(
                    icon_button("assistant", "sparkles", px(30.), true, cx)
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
                    .gap_0p5()
                    .overflow_x_scroll()
                    .children(chips),
            )
            .child(
                icon_button("settings", "settings", px(30.), true, cx).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.settings_open = !this.settings_open;
                        cx.notify();
                    },
                )),
            )
            .child(
                icon_button("new-menu", "plus", px(30.), true, cx).on_click(cx.listener(
                    |this, ev: &ClickEvent, _, cx| {
                        this.menu = Some(OpenMenu {
                            kind: MenuKind::NewMenu,
                            position: ev.position(),
                        });
                        cx.notify();
                    },
                )),
            )
    }
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
