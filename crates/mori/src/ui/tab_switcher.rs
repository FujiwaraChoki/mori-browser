//! Ctrl+Tab switches tabs in most-recently-used order (Arc): hold Ctrl,
//! press Tab to move through an overlay of recent tabs, release Ctrl to go
//! to the highlighted one. Resting the pointer on a tab row shows a hover
//! card with its title, address and a thumbnail. Both use [`Thumbnails`],
//! small copies of each page's last painted frame.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use mori_core::{
    tab_switcher::{RecentTabs, Switcher},
    util::host_of,
    workspace::TabId,
};
use smallvec::SmallVec;

use crate::{
    browser::Browser,
    theme::{ActiveTokens, metrics},
    ui::components::{favicon, h_flex, v_flex},
};

/// Width of thumbnails in device pixels (crisp at 2x for the ~180pt cards).
const THUMB_WIDTH: u32 = 360;
/// A quick Ctrl+Tab tap switches without flashing the overlay.
const REVEAL_DELAY: Duration = Duration::from_millis(110);
/// How long the pointer rests on a tab row before its card appears.
const HOVER_DELAY: Duration = Duration::from_millis(600);

const CARD_WIDTH: Pixels = px(168.);
const CARD_THUMB_HEIGHT: Pixels = px(104.);
const MAX_COLUMNS: usize = 6;
const HOVER_WIDTH: Pixels = px(264.);
const HOVER_THUMB_HEIGHT: Pixels = px(150.);

/// Downscaled last frames of tabs, keyed by the frame they were made from.
#[derive(Default)]
pub struct Thumbnails {
    map: HashMap<TabId, (u64, Arc<RenderImage>)>,
    /// Tabs whose thumbnail is being made.
    pending: HashSet<TabId>,
}

/// An open Ctrl+Tab switcher.
pub struct TabSwitcher {
    pub switcher: Switcher,
    /// Shown after [`REVEAL_DELAY`] while Ctrl is still held.
    visible: bool,
    _reveal: Task<()>,
}

/// The tab row under the pointer and its card.
#[derive(Default)]
pub struct TabHover {
    tab: Option<TabId>,
    /// Window y of the pointer when it reached the row.
    y: f32,
    shown: bool,
    _timer: Option<Task<()>>,
}

impl Browser {
    /// Seed tab recency from the restored session.
    pub fn init_recent_tabs(&mut self) {
        let tabs: Vec<(TabId, f64)> = self.ws.tabs.iter().map(|t| (t.id, t.last_accessed)).collect();
        self.recent_tabs = RecentTabs::seeded(&tabs);
    }

    /// A tab was just selected: it is the most recent now, and a hover card
    /// for it has served its purpose.
    pub fn note_tab_selected(&mut self, id: TabId) {
        self.recent_tabs.touch(id);
        self.tab_hover = TabHover::default();
    }

    // ---- Thumbnails ----------------------------------------------------------

    pub fn thumbnail(&self, id: TabId) -> Option<Arc<RenderImage>> {
        self.thumbnails.map.get(&id).map(|(_, image)| image.clone())
    }

    /// Make fresh thumbnails for `tabs` whose page painted since their last
    /// one. The frame is copied here; scaling runs off the UI thread.
    pub fn refresh_thumbnails(&mut self, tabs: &[TabId], cx: &mut Context<Self>) {
        for &id in tabs {
            let Some(live) = self.live.get(&id) else { continue };
            let surface = live.surface.read(cx);
            let generation = surface.frame_generation();
            let current = self.thumbnails.map.get(&id).is_some_and(|(g, _)| *g == generation);
            if current || self.thumbnails.pending.contains(&id) {
                continue;
            }
            let Some((width, height, bgra)) = surface.snapshot_bgra() else { continue };
            self.thumbnails.pending.insert(id);
            let scaled = cx.background_spawn(async move { downscale(width, height, bgra) });
            cx.spawn(async move |this, cx| {
                let image = scaled.await;
                this.update(cx, |this, cx| {
                    this.thumbnails.pending.remove(&id);
                    let Some(image) = image else { return };
                    if let Some((_, old)) = this.thumbnails.map.insert(id, (generation, image)) {
                        cx.drop_image(old, None);
                    }
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
    }

    /// Forget thumbnails of tabs that were closed.
    fn prune_thumbnails(&mut self, cx: &mut Context<Self>) {
        let gone: Vec<TabId> = self
            .thumbnails
            .map
            .keys()
            .copied()
            .filter(|id| self.ws.tab(*id).is_none())
            .collect();
        for id in gone {
            if let Some((_, image)) = self.thumbnails.map.remove(&id) {
                cx.drop_image(image, None);
            }
        }
    }

    // ---- Ctrl+Tab ------------------------------------------------------------

    /// Ctrl+Tab (`direction` 1) or Ctrl+Shift+Tab (-1).
    pub fn switch_recent_tab(&mut self, direction: isize, window: &mut Window, cx: &mut Context<Self>) {
        if !self.settings.ctrl_tab_recent_order {
            self.cycle_tab(direction, window, cx);
            return;
        }
        if let Some(open) = self.tab_switcher.as_mut() {
            open.switcher.step(direction);
            cx.notify();
            return;
        }
        let ws = &self.ws;
        self.recent_tabs.retain(|id| ws.tab(id).is_some());
        // Favorites belong to no space but show with every one.
        let order = self.recent_tabs.order(&self.ws.ordered_in(self.ws.active_space));
        let Some(switcher) = Switcher::start(order, self.active_tab(), direction) else { return };
        if !window.modifiers().control {
            // Not held (a remapped key, or Ctrl already let go): switch now.
            let id = switcher.selected();
            self.select_tab(id, window, cx);
            return;
        }
        self.prune_thumbnails(cx);
        self.refresh_thumbnails(switcher.tabs(), cx);
        self.tab_hover = TabHover::default();
        let reveal = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(REVEAL_DELAY).await;
            this.update(cx, |this, cx| {
                if let Some(open) = this.tab_switcher.as_mut() {
                    open.visible = true;
                    cx.notify();
                }
            })
            .ok();
        });
        self.tab_switcher = Some(TabSwitcher {
            switcher,
            visible: false,
            _reveal: reveal,
        });
        // Keys go to the switcher, not the page, until it closes.
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    /// Ctrl was released: go to the highlighted tab.
    pub fn commit_tab_switcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = self.tab_switcher.take() else { return };
        let id = open.switcher.selected();
        if self.ws.tab(id).is_some() && self.active_tab() != Some(id) {
            self.select_tab(id, window, cx);
        } else {
            self.focus_selected_page(window, cx);
        }
        cx.notify();
    }

    /// Escape: close the switcher and stay on the current tab.
    pub fn cancel_tab_switcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tab_switcher.take().is_some() {
            self.focus_selected_page(window, cx);
            cx.notify();
        }
    }

    pub fn on_switcher_modifiers(&mut self, ev: &ModifiersChangedEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.tab_switcher.is_some() && !ev.modifiers.control {
            self.commit_tab_switcher(window, cx);
        }
    }

    /// Keys while the switcher is open (Ctrl is held, so these arrive as
    /// Ctrl+key and match no binding).
    pub fn on_switcher_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = self.tab_switcher.as_mut() else { return };
        match ev.keystroke.key.as_str() {
            "escape" => self.cancel_tab_switcher(window, cx),
            "enter" => self.commit_tab_switcher(window, cx),
            "right" | "down" => {
                open.switcher.step(1);
                cx.notify();
            }
            "left" | "up" => {
                open.switcher.step(-1);
                cx.notify();
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    pub fn render_tab_switcher(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let open = self.tab_switcher.as_ref().filter(|o| o.visible)?;
        let t = cx.tokens().clone();
        let tabs = open.switcher.tabs().to_vec();
        let highlighted = open.switcher.index();
        let viewport = window.viewport_size();
        let columns = tabs.len().min(MAX_COLUMNS);
        let gap = px(10.);
        let padding = px(14.);
        let width = CARD_WIDTH * columns as f32 + gap * (columns as f32 - 1.) + padding * 2.;
        let (panel_bg, border): (Hsla, Hsla) = if t.dark {
            (rgb(0x181818).into(), rgb(0x4b4b4b).into())
        } else {
            (white(), t.popover_border())
        };
        let cards: Vec<AnyElement> = tabs
            .iter()
            .enumerate()
            .map(|(i, id)| self.render_switcher_card(i, *id, i == highlighted, cx))
            .collect();
        let mut grid = v_flex().gap(gap);
        let mut cards = cards.into_iter().peekable();
        while cards.peek().is_some() {
            grid = grid.child(h_flex().gap(gap).items_start().children(cards.by_ref().take(MAX_COLUMNS)));
        }
        let left = ((viewport.width - width) / 2.).max(px(8.));
        Some(
            div()
                .absolute()
                .inset_0()
                .occlude()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .id("tab-switcher")
                        .absolute()
                        .left(left)
                        .top((viewport.height * 0.28).max(px(16.)))
                        .w(width)
                        .p(padding)
                        .bg(panel_bg)
                        .rounded(metrics::RADIUS_POPOVER)
                        .border_1()
                        .border_color(border)
                        .shadow(t.modal_shadow())
                        .child(grid)
                        .with_animation(
                            "tab-switcher-open",
                            Animation::new(Duration::from_millis(120)).with_easing(ease_out_quint()),
                            |el, delta| el.opacity(delta),
                        ),
                )
                .into_any_element(),
        )
    }

    fn render_switcher_card(&mut self, i: usize, id: TabId, highlighted: bool, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.tokens().clone();
        let record = self.ws.tab(id);
        let title: SharedString = record.map(|r| r.display_title().to_string()).unwrap_or_default().into();
        let host = record.and_then(|r| host_of(&r.url));
        let fav = host.as_deref().and_then(|h| self.favicons.get(h));
        let thumb = self.thumbnail(id);
        let preview: AnyElement = match thumb {
            Some(image) => img(image)
                .size_full()
                .object_fit(ObjectFit::Cover)
                .rounded(metrics::RADIUS_LG)
                .into_any_element(),
            None => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .rounded(metrics::RADIUS_LG)
                .bg(t.tile_rest())
                .child(favicon(fav.clone(), px(28.), false, cx))
                .into_any_element(),
        };
        let ring = if highlighted { t.primary } else { transparent_black() };
        v_flex()
            .id(("switcher-card", i))
            .w(CARD_WIDTH)
            .gap(px(7.))
            .p(px(4.))
            .rounded(metrics::RADIUS)
            .border_2()
            .border_color(ring)
            .when(highlighted, |s| s.bg(t.ghost_pressed()))
            .cursor_pointer()
            .child(
                div()
                    .w_full()
                    .h(CARD_THUMB_HEIGHT)
                    .rounded(metrics::RADIUS_LG)
                    .overflow_hidden()
                    .bg(t.card)
                    .border_1()
                    .border_color(t.border.opacity(0.5))
                    .child(preview),
            )
            .child(
                h_flex()
                    .gap(px(6.))
                    .px(px(2.))
                    .child(favicon(fav, px(14.), false, cx))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(metrics::TEXT_LABEL)
                            .text_color(if highlighted { t.foreground } else { t.foreground.opacity(0.8) })
                            .child(title),
                    ),
            )
            .on_mouse_move(cx.listener(move |this, _, _, cx| {
                if let Some(open) = this.tab_switcher.as_mut()
                    && open.switcher.selected() != id
                {
                    open.switcher.highlight(id);
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |this, _, window, cx| {
                if let Some(open) = this.tab_switcher.as_mut() {
                    open.switcher.highlight(id);
                }
                this.commit_tab_switcher(window, cx);
            }))
            .into_any_element()
    }

    // ---- Hover cards -----------------------------------------------------------

    /// The pointer entered (`hovered`) or left a tab row.
    pub fn hover_tab_row(&mut self, id: TabId, hovered: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !hovered {
            if self.tab_hover.tab == Some(id) {
                let shown = self.tab_hover.shown;
                self.tab_hover = TabHover::default();
                if shown {
                    cx.notify();
                }
            }
            return;
        }
        let y = f32::from(window.mouse_position().y);
        let timer = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(HOVER_DELAY).await;
            this.update(cx, |this, cx| {
                let busy = this.menu.is_some()
                    || this.rename.is_some()
                    || this.tab_switcher.is_some()
                    || this.launcher.is_some()
                    || cx.has_active_drag();
                if this.tab_hover.tab != Some(id) || busy {
                    return;
                }
                this.tab_hover.shown = true;
                this.refresh_thumbnails(&[id], cx);
                cx.notify();
            })
            .ok();
        });
        self.tab_hover = TabHover {
            tab: Some(id),
            y,
            shown: false,
            _timer: Some(timer),
        };
    }

    pub fn render_tab_hover(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let id = self.tab_hover.tab.filter(|_| self.tab_hover.shown)?;
        // The row may have gone without a hover-out (Ctrl+S, compact mode,
        // the floating sidebar sliding away), or something covers it.
        let sidebar_shown = self.settings.docks_sidebar() || (self.sidebar_peek && !self.sidebar_fx.peek_leaving());
        let covered = self.menu.is_some()
            || self.launcher.is_some()
            || self.tab_switcher.is_some()
            || self.cheatsheet.is_some()
            || self.capture.is_some()
            || self.onboarding.is_some()
            || self.selected_live().is_some_and(|l| l.fullscreen);
        if !sidebar_shown || covered || cx.has_active_drag() {
            return None;
        }
        let record = self.ws.tab(id)?.clone();
        let t = cx.tokens().clone();
        let viewport = window.viewport_size();
        let thumb = self.thumbnail(id);
        let height = if thumb.is_some() { HOVER_THUMB_HEIGHT + px(64.) } else { px(64.) };
        let sidebar = px(self.sidebar_drag.unwrap_or(self.settings.clamped_sidebar_width())) + px(14.);
        let left = match self.settings.sidebar_position {
            mori_core::settings::SidebarPosition::Left => sidebar,
            mori_core::settings::SidebarPosition::Right => viewport.width - sidebar - HOVER_WIDTH,
        };
        let top = (px(self.tab_hover.y) - px(22.)).min(viewport.height - height - px(8.)).max(px(8.));
        let address = mori_core::omnibox::placeholder_title(&record.url);
        let asleep = !self.live.contains_key(&id) && !Browser::is_internal(&record.url);
        Some(
            v_flex()
                .absolute()
                .left(left)
                .top(top)
                .w(HOVER_WIDTH)
                .p(px(6.))
                .gap(px(6.))
                .bg(t.popover)
                .rounded(metrics::RADIUS_POPOVER)
                .border_1()
                .border_color(t.popover_border())
                .shadow(t.overlay_shadow())
                .when_some(thumb, |s, image| {
                    s.child(
                        div()
                            .w_full()
                            .h(HOVER_THUMB_HEIGHT)
                            .rounded(metrics::RADIUS)
                            .overflow_hidden()
                            .bg(t.card)
                            .child(img(image).size_full().object_fit(ObjectFit::Cover).rounded(metrics::RADIUS)),
                    )
                })
                .child(
                    v_flex()
                        .px(px(6.))
                        .pb(px(4.))
                        .gap(px(2.))
                        .child(
                            div()
                                .text_size(metrics::TEXT_BASE)
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(t.foreground)
                                .line_clamp(2)
                                .child(record.display_title().to_string()),
                        )
                        .child(
                            h_flex()
                                .gap(px(6.))
                                .text_size(metrics::TEXT_SMALL)
                                .text_color(t.muted_foreground)
                                .child(div().flex_1().min_w_0().truncate().child(address))
                                .when(asleep, |s| s.child(div().flex_none().child("Sleeping"))),
                        ),
                )
                .with_animation(
                    ("tab-hover", id.as_u128() as usize),
                    Animation::new(Duration::from_millis(140)).with_easing(ease_out_quint()),
                    |el, delta| el.opacity(delta),
                )
                .into_any_element(),
        )
    }
}

/// A `THUMB_WIDTH`-wide copy of a BGRA frame (the channel order is kept:
/// GPUI images are BGRA too).
fn downscale(width: u32, height: u32, bgra: Vec<u8>) -> Option<Arc<RenderImage>> {
    let frame = image::RgbaImage::from_raw(width, height, bgra)?;
    let w = THUMB_WIDTH.min(width).max(1);
    let h = ((height as u64 * w as u64) / width.max(1) as u64).max(1) as u32;
    let small = image::imageops::thumbnail(&frame, w, h);
    let frames: SmallVec<[image::Frame; 1]> = SmallVec::from_elem(image::Frame::new(small), 1);
    Some(Arc::new(RenderImage::new(frames)))
}
