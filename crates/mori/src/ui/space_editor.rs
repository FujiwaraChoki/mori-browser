//! The space editor: name, icon (any Lucide symbol or an emoji), and theme
//! (Arc-style colour wheel on the shared gradient engine), shown when a
//! space is created and from a space's menu. Changes apply live, so the
//! sidebar and wash behind the editor show the result as it is made.

use std::{
    cell::Cell,
    rc::Rc,
    sync::{Arc, OnceLock},
};

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AssetSource,
    component::{
        Sizable as _,
        input::{Escape, Input, InputEvent, InputState},
    },
    *,
};
use mori_core::{
    theme::{GradientDot, GradientTheme, Harmony, Rgba, harmony_dots, presets},
    workspace::{NEW_SPACE_COLORS, Space, SpaceId, space_theme_for_colors},
};
use smallvec::SmallVec;

use crate::{
    theme::{ActiveTokens, hsla, metrics},
    ui::{
        components::{h_flex, icon, popover_surface, text_button, v_flex},
        sidebar::{SPACE_EMOJI, SPACE_SYMBOLS, space_glyph},
    },
};

/// What the editor currently says about the space.
#[derive(Clone, Debug)]
pub struct SpaceUpdate {
    pub id: SpaceId,
    pub name: String,
    pub symbol: String,
    pub theme: GradientTheme,
}

pub enum SpaceEditorEvent {
    /// Apply this to the space now (live preview).
    Changed(SpaceUpdate),
    Done,
    /// Cancel creating this new space: remove it again.
    Discard(SpaceId),
}

#[derive(Clone, Copy, PartialEq)]
enum SymbolTab {
    Icons,
    Emoji,
}

/// Side of the colour wheel in logical pixels.
const WHEEL: f32 = 168.;
/// Grid cell for icon and emoji choices.
const CELL: f32 = 32.;
/// Icon search results shown at once.
const MAX_RESULTS: usize = 48;
/// A press this close to a dot (in px) grabs it instead of placing one.
const GRAB_RADIUS: f32 = 12.;

const WASH_STEPS: [(&str, f32); 4] = [("Soft", 0.35), ("Medium", 0.5), ("Strong", 0.7), ("Vivid", 0.9)];
const GRAIN_STEPS: [(&str, f32); 3] = [("Off", 0.0), ("Light", 0.25), ("Heavy", 0.6)];
const TONE_STEPS: [(&str, f32); 5] = [("Deep", 30.), ("Dark", 40.), ("Mid", 50.), ("Light", 62.), ("Pastel", 76.)];
const HARMONIES: [Harmony; 4] = [Harmony::Floating, Harmony::SingleAnalogous, Harmony::Complementary, Harmony::Triadic];

pub struct SpaceEditor {
    space: SpaceId,
    is_new: bool,
    name: Entity<InputState>,
    search: Entity<InputState>,
    emoji: Entity<InputState>,
    symbol: String,
    tab: SymbolTab,
    theme: GradientTheme,
    harmony: Harmony,
    /// HSL lightness (0…100) for dots placed on the wheel.
    lightness: f32,
    /// The dot being dragged on the wheel.
    dragging: Option<usize>,
    /// Where the wheel was painted, for turning pointer positions into
    /// wheel coordinates.
    wheel_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SpaceEditorEvent> for SpaceEditor {}

impl SpaceEditor {
    pub fn new(space: &Space, is_new: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("Space name");
            s.set_value(space.name.clone(), window, cx);
            s
        });
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search all icons"));
        let emoji = cx.new(|cx| InputState::new(window, cx).placeholder("Or type any emoji"));
        name.update(cx, |s, cx| {
            s.focus(window, cx);
            s.select_all(window, cx);
        });
        let subscriptions = vec![
            cx.subscribe_in(&name, window, |this, _, ev: &InputEvent, _, cx| match ev {
                InputEvent::Change => this.changed(cx),
                InputEvent::PressEnter { .. } => cx.emit(SpaceEditorEvent::Done),
                _ => {}
            }),
            cx.subscribe_in(&search, window, |_, _, ev: &InputEvent, _, cx| {
                if matches!(ev, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.subscribe_in(&emoji, window, |this, input, ev: &InputEvent, _, cx| {
                if matches!(ev, InputEvent::Change) {
                    let typed = typed_emoji(input.read(cx).value().as_ref());
                    if !typed.is_empty() {
                        this.symbol = typed;
                        this.changed(cx);
                    }
                }
            }),
        ];
        // The editor keeps the primary dot first so the wheel can treat it
        // as the one the others follow.
        let mut theme = space.theme.clone();
        if let Some(i) = theme.dots.iter().position(|d| d.is_primary)
            && i > 0
        {
            let dot = theme.dots.remove(i);
            theme.dots.insert(0, dot);
        }
        let lightness = theme
            .dots
            .first()
            .and_then(|d| d.lightness)
            .unwrap_or(50.);
        let tab = if mori_core::workspace::is_emoji_symbol(&space.symbol) { SymbolTab::Emoji } else { SymbolTab::Icons };
        SpaceEditor {
            space: space.id,
            is_new,
            name,
            search,
            emoji,
            symbol: space.symbol.clone(),
            tab,
            theme,
            harmony: Harmony::Floating,
            lightness,
            dragging: None,
            wheel_bounds: Rc::new(Cell::new(None)),
            _subscriptions: subscriptions,
        }
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        cx.emit(SpaceEditorEvent::Changed(SpaceUpdate {
            id: self.space,
            name: self.name.read(cx).value().to_string(),
            symbol: self.symbol.clone(),
            theme: self.theme.clone(),
        }));
        cx.notify();
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        if self.is_new {
            cx.emit(SpaceEditorEvent::Discard(self.space));
        } else {
            cx.emit(SpaceEditorEvent::Done);
        }
    }

    // ---- Theme ----------------------------------------------------------------

    fn set_theme(&mut self, theme: GradientTheme, harmony: Harmony, cx: &mut Context<Self>) {
        self.lightness = theme.dots.first().and_then(|d| d.lightness).unwrap_or(self.lightness);
        self.theme = theme;
        self.harmony = harmony;
        self.changed(cx);
    }

    /// Put the primary dot at wheel point (`x`, `y`); with a harmony the
    /// other dots follow it.
    fn place_primary(&mut self, x: f32, y: f32) {
        let primary = GradientDot::from_wheel(x, y, Some(self.lightness), true);
        match self.harmony {
            Harmony::Floating => match self.theme.dots.first_mut() {
                Some(first) => *first = primary,
                None => self.theme.dots.push(primary),
            },
            harmony => {
                let mut dots = vec![primary.clone()];
                dots.extend(harmony_dots(&primary, harmony));
                self.theme.dots = dots;
            }
        }
        self.theme.preset_id = None;
    }

    fn move_dot(&mut self, index: usize, x: f32, y: f32) {
        if index == 0 || self.harmony != Harmony::Floating {
            self.place_primary(x, y);
        } else if let Some(dot) = self.theme.dots.get_mut(index) {
            *dot = GradientDot::from_wheel(x, y, Some(self.lightness), false);
            self.theme.preset_id = None;
        }
    }

    /// Wheel coordinates (0…1, clamped into the circle) of a window point.
    fn wheel_point(&self, position: Point<Pixels>) -> Option<(f32, f32)> {
        let b = self.wheel_bounds.get()?;
        let x = f32::from(position.x - b.origin.x) / f32::from(b.size.width);
        let y = f32::from(position.y - b.origin.y) / f32::from(b.size.height);
        let (dx, dy) = (x - 0.5, y - 0.5);
        let r = dx.hypot(dy);
        Some(if r > 0.5 { (0.5 + dx * 0.5 / r, 0.5 + dy * 0.5 / r) } else { (x, y) })
    }

    fn wheel_down(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some((x, y)) = self.wheel_point(position) else { return };
        let grab = GRAB_RADIUS / WHEEL;
        let hit = self
            .theme
            .dots
            .iter()
            .position(|d| (d.x - x).hypot(d.y - y) <= grab);
        let index = match hit {
            Some(i) => i,
            None if self.harmony == Harmony::Floating
                && !self.theme.dots.is_empty()
                && self.theme.dots.len() < GradientTheme::MAX_DOTS =>
            {
                self.theme.dots.push(GradientDot::from_wheel(x, y, Some(self.lightness), false));
                self.theme.dots.len() - 1
            }
            None => 0,
        };
        self.dragging = Some(index);
        self.move_dot(index, x, y);
        self.changed(cx);
    }

    fn wheel_drag(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(index) = self.dragging else { return };
        if let Some((x, y)) = self.wheel_point(position) {
            self.move_dot(index, x, y);
            self.changed(cx);
        }
    }

    fn set_lightness(&mut self, lightness: f32, cx: &mut Context<Self>) {
        self.lightness = lightness;
        for (i, dot) in self.theme.dots.iter_mut().enumerate() {
            *dot = GradientDot::from_wheel(dot.x, dot.y, Some(lightness), i == 0);
        }
        self.theme.preset_id = None;
        self.changed(cx);
    }

    fn set_harmony(&mut self, harmony: Harmony, cx: &mut Context<Self>) {
        self.harmony = harmony;
        let (x, y) = self.theme.dots.first().map(|d| (d.x, d.y)).unwrap_or((0.8, 0.5));
        if harmony == Harmony::Floating {
            self.theme.dots.truncate(1);
        }
        self.place_primary(x, y);
        self.changed(cx);
    }

    // ---- Rendering --------------------------------------------------------------

    fn render_symbols(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.tokens().clone();
        let cell = |id: ElementId, active: bool| {
            div()
                .id(id)
                .size(px(CELL - 2.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .cursor_pointer()
                .when(active, |s| s.bg(t.primary.opacity(0.22)).border_1().border_color(t.primary.opacity(0.8)))
                .when(!active, |s| s.hover(|s| s.bg(t.foreground.opacity(0.08))))
        };
        let current = self.symbol.clone();
        let cells: Vec<AnyElement> = match self.tab {
            SymbolTab::Icons => {
                let query = self.search.read(cx).value().trim().to_lowercase();
                let names: Vec<String> = if query.is_empty() {
                    SPACE_SYMBOLS.iter().map(|s| s.to_string()).collect()
                } else {
                    all_icon_names()
                        .iter()
                        .filter(|n| n.contains(&query))
                        .take(MAX_RESULTS)
                        .cloned()
                        .collect()
                };
                names
                    .into_iter()
                    .enumerate()
                    .map(|(i, name)| {
                        let active = name == current;
                        let tip: SharedString = name.clone().into();
                        cell(("space-icon", i).into(), active)
                            .child(icon(&name, px(15.), if active { t.foreground } else { t.foreground.opacity(0.75) }))
                            .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(tip.clone()).build(window, cx))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.symbol = name.clone();
                                this.changed(cx);
                            }))
                            .into_any_element()
                    })
                    .collect()
            }
            SymbolTab::Emoji => SPACE_EMOJI
                .iter()
                .enumerate()
                .map(|(i, e)| {
                    let e = e.to_string();
                    let active = e == current;
                    cell(("space-emoji", i).into(), active)
                        .child(space_glyph(&e, px(16.), t.foreground))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.symbol = e.clone();
                            this.changed(cx);
                        }))
                        .into_any_element()
                })
                .collect(),
        };
        let empty = cells.is_empty();
        let field = match self.tab {
            SymbolTab::Icons => &self.search,
            SymbolTab::Emoji => &self.emoji,
        };
        v_flex()
            .gap(px(8.))
            .child(
                h_flex()
                    .gap(px(8.))
                    .child(segmented(
                        "symbol-tab",
                        &[("Icons", SymbolTab::Icons), ("Emoji", SymbolTab::Emoji)],
                        self.tab,
                        |this, tab, _, cx| {
                            this.tab = tab;
                            cx.notify();
                        },
                        cx,
                    ))
                    .child(div().flex_1().child(Input::new(field).small())),
            )
            .child(
                div()
                    .h(px(CELL * 3. + 4.))
                    .id("symbol-cells")
                    .overflow_y_scroll()
                    .child(div().grid().grid_cols(10).gap(px(2.)).children(cells))
                    .when(empty, |s| {
                        s.child(
                            div()
                                .pt(px(10.))
                                .text_size(metrics::TEXT_SMALL)
                                .text_color(t.muted_foreground)
                                .child("No icon matches"),
                        )
                    }),
            )
            .into_any_element()
    }

    fn render_wheel(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.tokens().clone();
        let bounds = self.wheel_bounds.clone();
        let image = wheel_image();
        let wheel = canvas(
            move |b, _, _| bounds.set(Some(b)),
            move |b, _, window, _| {
                let _ = window.paint_image(b, b, Corners::all(b.size.width / 2.), image.clone(), 0, false);
            },
        )
        .absolute()
        .inset_0();
        let dots: Vec<AnyElement> = self
            .theme
            .dots
            .iter()
            .enumerate()
            .map(|(i, d)| {
                let size = if i == 0 { 18. } else { 13. };
                div()
                    .absolute()
                    .left(px(d.x * WHEEL - size / 2.))
                    .top(px(d.y * WHEEL - size / 2.))
                    .size(px(size))
                    .rounded_full()
                    .border_2()
                    .border_color(white())
                    .bg(hsla(d.rgb))
                    .shadow(vec![BoxShadow {
                        color: black().opacity(0.35),
                        offset: point(px(0.), px(1.)),
                        blur_radius: px(3.),
                        spread_radius: px(0.),
                        inset: false,
                    }])
                    .into_any_element()
            })
            .collect();
        div()
            .id("space-wheel")
            .relative()
            .flex_none()
            .size(px(WHEEL))
            .rounded_full()
            .border_1()
            .border_color(t.border.opacity(0.6))
            .cursor_crosshair()
            .child(wheel)
            .children(dots)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.wheel_down(ev.position, cx);
                }),
            )
            .into_any_element()
    }

    fn render_theme(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.tokens().clone();
        let current = self.theme.clone();
        let swatch = |id: ElementId, colors: Vec<Rgba>, active: bool, tip: SharedString| {
            let fill: AnyElement = match colors.as_slice() {
                [] => div()
                    .size_full()
                    .rounded_full()
                    .bg(t.sidebar)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon("ban", px(11.), t.muted_foreground))
                    .into_any_element(),
                [one] => div().size_full().rounded_full().bg(hsla(*one)).into_any_element(),
                [a, b, ..] => div()
                    .size_full()
                    .rounded_full()
                    .bg(linear_gradient(
                        135.,
                        linear_color_stop(hsla(*a), 0.),
                        linear_color_stop(hsla(*b), 1.),
                    ))
                    .into_any_element(),
            };
            div()
                .id(id)
                .size(px(24.))
                .flex_none()
                .p(px(2.))
                .rounded_full()
                .border_2()
                .border_color(if active { t.foreground } else { transparent_black() })
                .cursor_pointer()
                .child(fill)
                .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(tip.clone()).build(window, cx))
        };
        let same_primary = |theme: &GradientTheme| {
            let hex = |th: &GradientTheme| th.dots.first().map(|d| d.rgb.with_alpha(1.0).to_hex_string());
            hex(theme) == hex(&current) && theme.dots.len() == current.dots.len()
        };
        let mut swatches: Vec<AnyElement> = vec![
            swatch("swatch-none".into(), Vec::new(), current.is_none(), "No theme".into())
                .on_click(cx.listener(|this, _, _, cx| this.set_theme(GradientTheme::none(), Harmony::Floating, cx)))
                .into_any_element(),
        ];
        for (i, c) in NEW_SPACE_COLORS.iter().enumerate() {
            let theme = space_theme_for_colors(*c);
            let active = current.preset_id.is_none() && same_primary(&theme);
            let colors = theme.colors();
            swatches.push(
                swatch(("swatch", i).into(), colors, active, "Colour".into())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_theme(theme.clone(), Harmony::Floating, cx)
                    }))
                    .into_any_element(),
            );
        }
        for (i, p) in presets().into_iter().enumerate() {
            let active = current.preset_id.as_deref() == Some(p.id);
            let colors = p.colors.iter().map(|c| Rgba::hex(*c)).collect();
            let tip: SharedString = format!("{} · {}", p.name, p.subtitle).into();
            let theme = p.theme();
            swatches.push(
                swatch(("preset", i).into(), colors, active, tip)
                    .on_click(cx.listener(move |this, _, _, cx| this.set_theme(theme.clone(), Harmony::Floating, cx)))
                    .into_any_element(),
            );
        }
        let has_theme = !current.is_none();
        let nearest = |steps: &[(&str, f32)], value: f32| {
            steps
                .iter()
                .map(|(_, v)| *v)
                .min_by(|a, b| (a - value).abs().total_cmp(&(b - value).abs()))
                .unwrap_or(value)
        };
        let wash = nearest(&WASH_STEPS, current.opacity);
        let grain = nearest(&GRAIN_STEPS, current.texture);
        let tone = nearest(&TONE_STEPS, self.lightness);
        let label = |s: &'static str| {
            div()
                .w(px(64.))
                .flex_none()
                .text_size(metrics::TEXT_SMALL)
                .text_color(t.muted_foreground)
                .child(s)
        };
        let harmonies: Vec<(&'static str, Harmony)> = HARMONIES.iter().map(|h| (h.label(), *h)).collect();
        h_flex()
            .items_start()
            .gap(px(16.))
            .child(self.render_wheel(cx))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(10.))
                    .child(h_flex().flex_wrap().gap(px(4.)).children(swatches))
                    .when(has_theme, |s| {
                        s.child(
                            v_flex()
                                .gap(px(6.))
                                .child(h_flex().gap(px(6.)).child(label("Colours")).child(segmented(
                                    "harmony",
                                    &harmonies,
                                    self.harmony,
                                    |this, h, _, cx| this.set_harmony(h, cx),
                                    cx,
                                )))
                                .child(h_flex().gap(px(6.)).child(label("Tone")).child(segmented(
                                    "tone",
                                    &TONE_STEPS,
                                    tone,
                                    |this, v, _, cx| this.set_lightness(v, cx),
                                    cx,
                                )))
                                .child(h_flex().gap(px(6.)).child(label("Wash")).child(segmented(
                                    "wash",
                                    &WASH_STEPS,
                                    wash,
                                    |this, v, _, cx| {
                                        this.theme.opacity = v;
                                        this.changed(cx);
                                    },
                                    cx,
                                )))
                                .child(h_flex().gap(px(6.)).child(label("Grain")).child(segmented(
                                    "grain",
                                    &GRAIN_STEPS,
                                    grain,
                                    |this, v, _, cx| {
                                        this.theme.texture = v;
                                        this.changed(cx);
                                    },
                                    cx,
                                ))),
                        )
                    })
                    .child(
                        div()
                            .text_size(metrics::TEXT_SMALL)
                            .text_color(t.muted_foreground)
                            .child(if self.harmony == Harmony::Floating {
                                "Click the wheel to add up to three colours; drag them to change."
                            } else {
                                "Drag the big dot; the others follow it."
                            }),
                    ),
            )
            .into_any_element()
    }
}

impl Render for SpaceEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens().clone();
        let section = |s: &'static str| {
            div()
                .text_size(metrics::TEXT_SMALL)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.muted_foreground)
                .child(s.to_uppercase())
        };
        // The space's chip as it will look: its colours behind its glyph.
        let colors = self.theme.colors();
        let tile_bg: Background = match colors.as_slice() {
            [] => t.tile_rest().into(),
            [one] => hsla(*one).into(),
            [a, b, ..] => linear_gradient(135., linear_color_stop(hsla(*a), 0.), linear_color_stop(hsla(*b), 1.)),
        };
        let glyph_color = match colors.first() {
            Some(c) => hsla(mori_core::theme::contrasting_text(*c)),
            None => t.foreground,
        };
        let symbols = self.render_symbols(cx);
        let theme = self.render_theme(cx);
        let footer = if self.is_new {
            h_flex()
                .gap_2()
                .child(text_button("space-cancel", "Cancel", false, cx).on_click(cx.listener(|this, _, _, cx| this.cancel(cx))))
                .child(
                    text_button("space-create", "Create Space", true, cx)
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(SpaceEditorEvent::Done))),
                )
        } else {
            h_flex().child(
                text_button("space-done", "Done", true, cx).on_click(cx.listener(|_, _, _, cx| cx.emit(SpaceEditorEvent::Done))),
            )
        };
        div()
            .absolute()
            .inset_0()
            .occlude()
            .bg(black().opacity(0.28))
            .flex()
            // A fixed top, so picking "No theme" (which hides the theme
            // controls) doesn't move everything under the pointer.
            .items_start()
            .pt(px(72.))
            .justify_center()
            .on_mouse_down(MouseButton::Left, cx.listener(|_, _, _, cx| cx.emit(SpaceEditorEvent::Done)))
            // Dragging a wheel dot may stray outside the wheel.
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                if ev.pressed_button == Some(MouseButton::Left) {
                    this.wheel_drag(ev.position, cx);
                } else {
                    this.dragging = None;
                }
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, _| this.dragging = None))
            .child(
                popover_surface(cx)
                    .id("space-editor")
                    .w(px(560.))
                    .shadow(t.modal_shadow())
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .capture_action(cx.listener(|this, _: &Escape, _, cx| {
                        this.cancel(cx);
                        cx.stop_propagation();
                    }))
                    .child(
                        v_flex()
                            .px_4()
                            .pt_4()
                            .pb_3()
                            .gap(px(2.))
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_size(metrics::TEXT_TITLE)
                                    .child(if self.is_new { "Create a Space" } else { "Edit Space" }),
                            )
                            .child(
                                div()
                                    .text_size(metrics::TEXT_SMALL)
                                    .text_color(t.muted_foreground)
                                    .child("A space keeps its own pinned tabs, folders and colours."),
                            ),
                    )
                    .child(
                        v_flex()
                            .px_4()
                            .pb_4()
                            .gap(px(14.))
                            .child(
                                h_flex()
                                    .gap(px(12.))
                                    .child(
                                        div()
                                            .size(px(40.))
                                            .flex_none()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .rounded(metrics::RADIUS)
                                            .bg(tile_bg)
                                            .child(space_glyph(&self.symbol, px(18.), glyph_color)),
                                    )
                                    .child(div().flex_1().child(Input::new(&self.name))),
                            )
                            .child(v_flex().gap(px(8.)).child(section("Icon")).child(symbols))
                            .child(v_flex().gap(px(8.)).child(section("Theme")).child(theme)),
                    )
                    .child(
                        h_flex()
                            .px_4()
                            .py_3()
                            .border_t_1()
                            .border_color(t.border.opacity(0.6))
                            .child(div().flex_1())
                            .child(footer),
                    ),
            )
    }
}

/// A small segmented control: one button per choice, the current one lit.
fn segmented<T: Copy + PartialEq + 'static>(
    id: &'static str,
    choices: &[(&'static str, T)],
    current: T,
    on_pick: impl Fn(&mut SpaceEditor, T, &mut Window, &mut Context<SpaceEditor>) + 'static,
    cx: &mut Context<SpaceEditor>,
) -> impl IntoElement {
    let t = cx.tokens().clone();
    let on_pick = Rc::new(on_pick);
    h_flex()
        .p(px(2.))
        .gap(px(2.))
        .rounded(metrics::RADIUS_LG)
        .bg(t.foreground.opacity(0.06))
        .children(choices.iter().enumerate().map(|(i, (label, value))| {
            let value = *value;
            let active = value == current;
            let on_pick = on_pick.clone();
            div()
                .id((id, i))
                .h(px(22.))
                .px(px(8.))
                .flex()
                .items_center()
                .rounded(metrics::RADIUS_MD)
                .text_size(metrics::TEXT_SMALL)
                .cursor_pointer()
                .when(active, |s| s.bg(t.popover).shadow(t.popover_shadow()).text_color(t.foreground))
                .when(!active, |s| s.text_color(t.muted_foreground).hover(|s| s.text_color(t.foreground)))
                .child(*label)
                .on_click(cx.listener(move |this, _, window, cx| on_pick(this, value, window, cx)))
        }))
}

/// The emoji at the start of what was typed (up to the first ASCII
/// character), so stray letters don't end up in the switcher.
fn typed_emoji(value: &str) -> String {
    value.trim().chars().take_while(|c| !c.is_ascii()).take(8).collect()
}

/// Every Lucide icon name, sorted, for the icon search.
fn all_icon_names() -> &'static [String] {
    static NAMES: OnceLock<Vec<String>> = OnceLock::new();
    NAMES.get_or_init(|| {
        let mut names: Vec<String> = crate::assets::MoriAssets
            .list("icons/")
            .unwrap_or_default()
            .iter()
            .filter_map(|p| p.strip_prefix("icons/")?.strip_suffix(".svg").map(str::to_string))
            .collect();
        names.sort();
        names.dedup();
        names
    })
}

/// The hue/saturation wheel at mid lightness, drawn once.
fn wheel_image() -> Arc<RenderImage> {
    static IMAGE: OnceLock<Arc<RenderImage>> = OnceLock::new();
    IMAGE
        .get_or_init(|| {
            const SIDE: u32 = 2 * WHEEL as u32;
            let mut bytes = Vec::with_capacity((SIDE * SIDE * 4) as usize);
            for y in 0..SIDE {
                for x in 0..SIDE {
                    let (u, v) = ((x as f32 + 0.5) / SIDE as f32, (y as f32 + 0.5) / SIDE as f32);
                    let r = (u - 0.5).hypot(v - 0.5);
                    // Anti-aliased rim, one pixel wide.
                    let alpha = ((0.5 - r) * SIDE as f32 + 0.5).clamp(0.0, 1.0);
                    let c = mori_core::theme::dot_from_wheel(u, v, Some(50.));
                    let ch = |v: f32| (v.clamp(0.0, 1.0) * alpha * 255.0).round() as u8;
                    // Premultiplied BGRA.
                    bytes.extend_from_slice(&[ch(c.b), ch(c.g), ch(c.r), (alpha * 255.0).round() as u8]);
                }
            }
            let buffer = image::RgbaImage::from_raw(SIDE, SIDE, bytes).expect("wheel buffer size");
            let frames: SmallVec<[image::Frame; 1]> = SmallVec::from_elem(image::Frame::new(buffer), 1);
            Arc::new(RenderImage::new(frames))
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::typed_emoji;

    #[test]
    fn typed_emoji_keeps_only_the_emoji() {
        assert_eq!(typed_emoji(" 🚀 "), "🚀");
        assert_eq!(typed_emoji("❤️abc"), "❤️");
        assert_eq!(typed_emoji("abc"), "");
        assert_eq!(typed_emoji("👩‍💻"), "👩‍💻");
    }
}
