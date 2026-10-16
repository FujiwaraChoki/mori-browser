//! Mori's design tokens for GPUI.
//!
//! Colours come from `mori_core::theme` (the shared palette and gradient
//! engine); this module converts them to GPUI colours, keeps them in a
//! global, and mirrors them into GPUI Component's theme so stock inputs and
//! menus match the chrome.

use std::sync::{Arc, OnceLock};

use gpui_kit::{component::Theme as KitTheme, component::ThemeMode, *};
use mori_core::theme::{self as core_theme, GradientTheme, Palette, Rgba};
use smallvec::SmallVec;

/// Corner radii, spacing and type sizes from the original design system.
pub mod metrics {
    use gpui_kit::{Pixels, px};

    pub const RADIUS_MD: Pixels = px(4.4);
    pub const RADIUS_LG: Pixels = px(6.4);
    pub const RADIUS: Pixels = px(10.);
    pub const RADIUS_POPOVER: Pixels = px(12.);

    pub const TEXT_BASE: Pixels = px(13.);
    pub const TEXT_LABEL: Pixels = px(12.);
    pub const TEXT_SMALL: Pixels = px(11.);
    pub const TEXT_CAPTION: Pixels = px(10.);
    pub const TEXT_TITLE: Pixels = px(15.);

    pub const SIDEBAR_DEFAULT: f32 = mori_core::settings::SIDEBAR_DEFAULT_WIDTH;
    pub const SIDEBAR_MIN: f32 = mori_core::settings::SIDEBAR_MIN_WIDTH;
    pub const SIDEBAR_MAX: f32 = mori_core::settings::SIDEBAR_MAX_WIDTH;
    pub const CARD_INSET: Pixels = px(8.);
    /// The web card's corner radius (Zen: ~6px, borderless).
    pub const CARD_RADIUS: Pixels = px(6.);

    /// Sidebar content inset from the window edge (address pill, pinned grid).
    pub const SIDEBAR_INSET: Pixels = px(7.);
    /// Extra inset of tab rows relative to the pill.
    pub const ROW_INSET: Pixels = px(2.);
    pub const ROW_HEIGHT: Pixels = px(36.);
    pub const ROW_GAP: Pixels = px(4.);
    pub const ROW_RADIUS: Pixels = px(8.);
    pub const PILL_HEIGHT: Pixels = px(34.);
    pub const TOOLBAR_BUTTON: Pixels = px(30.);
    pub const FAVICON: Pixels = px(16.);
}

/// The desktop's interface font, resolved once at startup.
pub struct UiFont {
    /// Family name as installed (after fontconfig substitution).
    pub family: SharedString,
    /// The interface font size: tab titles and sidebar rows (11pt = 14.67px).
    pub size: Pixels,
}

impl UiFont {
    /// Address pill text (Zen: ~15.4px at 11pt).
    pub fn url_size(&self) -> Pixels {
        self.size * 1.05
    }

    /// Space label above the tab list (Zen: 13px bold at 11pt).
    pub fn label_size(&self) -> Pixels {
        self.size * 0.886
    }
}

/// The system UI font: GNOME's `font-name` setting, resolved through
/// fontconfig so user substitutions apply exactly as they do for GTK apps
/// (Zen); `sans-serif` when the setting is unavailable.
pub fn ui_font() -> &'static UiFont {
    static FONT: OnceLock<UiFont> = OnceLock::new();
    FONT.get_or_init(|| {
        let output = |cmd: &str, args: &[&str]| {
            std::process::Command::new(cmd)
                .args(args)
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        };
        let (requested, points) = output("gsettings", &["get", "org.gnome.desktop.interface", "font-name"])
            .as_deref()
            .and_then(core_theme::parse_font_description)
            .unwrap_or_else(|| ("sans-serif".into(), core_theme::DEFAULT_UI_FONT_POINTS));
        let family = output("fc-match", &["-f", "%{family}", &requested])
            .as_deref()
            .and_then(core_theme::first_font_family)
            .unwrap_or_else(|| ".SystemUIFont".into());
        log::info!("UI font: {family} {points}pt (requested {requested})");
        UiFont {
            family: family.into(),
            size: px(core_theme::points_to_px(points)),
        }
    })
}

pub fn hsla(c: Rgba) -> Hsla {
    Rgba32 {
        r: c.r,
        g: c.g,
        b: c.b,
        a: c.a,
    }
    .into()
}

/// GPUI's `Rgba` under a local name so it doesn't clash with core's.
type Rgba32 = gpui_kit::Rgba;

#[derive(Clone)]
pub struct Tokens {
    pub dark: bool,
    pub background: Hsla,
    pub foreground: Hsla,
    pub card: Hsla,
    pub popover: Hsla,
    pub primary: Hsla,
    pub primary_foreground: Hsla,
    pub muted: Hsla,
    pub muted_foreground: Hsla,
    pub destructive: Hsla,
    pub border: Hsla,
    pub input: Hsla,
    pub ring: Hsla,
    pub sidebar: Hsla,
    pub sidebar_foreground: Hsla,
    pub sidebar_border: Hsla,
    pub status_info: Hsla,
    pub status_success: Hsla,
    pub status_warning: Hsla,
    /// The active space's gradient theme (empty = plain chrome).
    pub gradient: GradientTheme,
    /// Low-resolution render of the gradient wash, stretched over the window.
    pub wash: Option<Arc<RenderImage>>,
    /// 128×128 film-grain tile, drawn repeated over the wash.
    pub grain: Option<Arc<RenderImage>>,
}

impl Global for Tokens {}

impl Tokens {
    fn from_palette(p: &Palette, dark: bool, gradient: GradientTheme) -> Self {
        let wash = (!gradient.is_none()).then(|| render_wash(&gradient));
        let grain = (!gradient.is_none() && gradient.clamped_texture() > 0.0)
            .then(|| render_grain(gradient.clamped_texture()));
        Tokens {
            dark,
            background: hsla(p.background),
            foreground: hsla(p.foreground),
            card: hsla(p.card),
            popover: hsla(p.popover),
            primary: hsla(p.primary),
            primary_foreground: hsla(p.primary_foreground),
            muted: hsla(p.muted),
            muted_foreground: hsla(p.muted_foreground),
            destructive: hsla(p.destructive),
            border: hsla(p.border),
            input: hsla(p.input),
            ring: hsla(p.ring),
            sidebar: hsla(p.sidebar),
            sidebar_foreground: hsla(p.sidebar_foreground),
            sidebar_border: hsla(p.sidebar_border),
            status_info: hsla(p.status_info),
            status_success: hsla(p.status_success),
            status_warning: hsla(p.status_warning),
            gradient,
            wash,
            grain,
        }
    }

    // ---- Sidebar surface tokens (Zen) ---------------------------------------
    // Translucent so a space's gradient wash shows through.

    pub fn tab_selected(&self) -> Hsla {
        if self.dark {
            white().opacity(0.17)
        } else {
            white().opacity(0.72)
        }
    }

    /// The selected row's lift in light mode (dark rows sit flat, as in Zen).
    pub fn tab_selected_shadow(&self) -> Vec<BoxShadow> {
        if self.dark {
            return Vec::new();
        }
        vec![BoxShadow {
            color: black().opacity(0.08),
            offset: point(px(0.), px(1.)),
            blur_radius: px(2.),
            spread_radius: px(0.),
            inset: false,
        }]
    }

    pub fn tab_hover(&self) -> Hsla {
        if self.dark {
            white().opacity(0.07)
        } else {
            black().opacity(0.05)
        }
    }

    /// Resting fill of the address pill and the pinned tiles.
    pub fn tile_rest(&self) -> Hsla {
        if self.dark {
            white().opacity(0.12)
        } else {
            black().opacity(0.07)
        }
    }

    pub fn tile_hover(&self) -> Hsla {
        if self.dark {
            white().opacity(0.17)
        } else {
            black().opacity(0.10)
        }
    }

    /// Hairlines between sidebar sections.
    pub fn sidebar_separator(&self) -> Hsla {
        if self.dark {
            white().opacity(0.09)
        } else {
            black().opacity(0.10)
        }
    }

    /// `layer` composited over the plain sidebar, for overlays that must
    /// match a translucent surface exactly (title fade-outs). `None` while a
    /// gradient wash is showing, because the sidebar is not one colour then.
    pub fn opaque_on_sidebar(&self, layer: Hsla) -> Option<Hsla> {
        self.opaque_on_sidebar_layers(&[layer])
    }

    /// Like [`Self::opaque_on_sidebar`] for several layers painted bottom to
    /// top, e.g. a drop shadow that shows through a translucent row.
    pub fn opaque_on_sidebar_layers(&self, layers: &[Hsla]) -> Option<Hsla> {
        if !self.gradient.is_none() {
            return None;
        }
        let mut out = self.sidebar.to_rgb();
        for layer in layers {
            let top = layer.to_rgb();
            let a = top.a;
            let mix = |t: f32, b: f32| t * a + b * (1.0 - a);
            out = Rgba32 {
                r: mix(top.r, out.r),
                g: mix(top.g, out.g),
                b: mix(top.b, out.b),
                a: 1.0,
            };
        }
        out.a = 1.0;
        Some(out.into())
    }

    pub fn ghost_hover(&self) -> Hsla {
        self.foreground.opacity(0.05)
    }

    pub fn ghost_pressed(&self) -> Hsla {
        self.foreground.opacity(0.09)
    }

    /// Outline of menus, the launcher and other floating surfaces.
    pub fn popover_border(&self) -> Hsla {
        if self.dark {
            white().opacity(0.16)
        } else {
            black().opacity(0.10)
        }
    }

    /// Divider lines inside popovers.
    pub fn popover_separator(&self) -> Hsla {
        if self.dark {
            white().opacity(0.08)
        } else {
            black().opacity(0.08)
        }
    }

    pub fn shadow_alpha(&self, light: f32, dark: f32) -> f32 {
        if self.dark { dark } else { light }
    }

    /// The web card's drop shadow: barely there on dark, a soft halo on light.
    pub fn card_shadow(&self) -> Vec<BoxShadow> {
        let (alpha, y, blur) = if self.dark { (0.25, 1., 4.) } else { (0.10, 0., 10.) };
        vec![BoxShadow {
            color: black().opacity(alpha),
            offset: point(px(0.), px(y)),
            blur_radius: px(blur),
            spread_radius: px(0.),
            inset: false,
        }]
    }

    pub fn popover_shadow(&self) -> Vec<BoxShadow> {
        vec![BoxShadow {
            color: black().opacity(self.shadow_alpha(0.18, 0.50)),
            offset: point(px(0.), px(5.)),
            blur_radius: px(14.),
            spread_radius: px(0.),
            inset: false,
        }]
    }

    pub fn overlay_shadow(&self) -> Vec<BoxShadow> {
        vec![BoxShadow {
            color: black().opacity(self.shadow_alpha(0.18, 0.45)),
            offset: point(px(0.), px(8.)),
            blur_radius: px(24.),
            spread_radius: px(0.),
            inset: false,
        }]
    }

    pub fn modal_shadow(&self) -> Vec<BoxShadow> {
        vec![BoxShadow {
            color: black().opacity(self.shadow_alpha(0.25, 0.60)),
            offset: point(px(0.), px(22.)),
            blur_radius: px(44.),
            spread_radius: px(0.),
            inset: false,
        }]
    }
}

pub trait ActiveTokens {
    fn tokens(&self) -> &Tokens;
}

impl ActiveTokens for App {
    fn tokens(&self) -> &Tokens {
        self.global::<Tokens>()
    }
}

/// Recompute tokens for the current appearance and space theme, and mirror
/// them into GPUI Component's theme.
pub fn apply(
    appearance: mori_core::settings::Appearance,
    gradient: &GradientTheme,
    window: Option<&mut Window>,
    cx: &mut App,
) {
    use mori_core::settings::Appearance;
    let system_dark = window
        .as_ref()
        .map(|w| matches!(w.appearance(), WindowAppearance::Dark | WindowAppearance::VibrantDark))
        .unwrap_or(true);
    let appearance_dark = match appearance {
        Appearance::System => system_dark,
        Appearance::Light => false,
        Appearance::Dark => true,
    };
    let dark = core_theme::effective_dark(gradient, appearance_dark);
    let base = if dark { Palette::dark() } else { Palette::light() };
    let palette = core_theme::themed_palette(base, gradient, dark);

    if let Some(old) = cx.try_global::<Tokens>() {
        for image in [old.wash.clone(), old.grain.clone()].into_iter().flatten() {
            cx.drop_image(image, None);
        }
    }
    let tokens = Tokens::from_palette(&palette, dark, gradient.clone());

    // Pages follow the chrome, so dark-aware sites (and their favicons)
    // render dark while Mori is dark.
    mori_engine::set_color_scheme(dark);

    KitTheme::change(if dark { ThemeMode::Dark } else { ThemeMode::Light }, window, cx);
    let kit = KitTheme::global_mut(cx);
    kit.font_family = ui_font().family.clone();
    kit.font_size = px(13.);
    kit.radius = metrics::RADIUS_MD;
    kit.radius_lg = metrics::RADIUS;
    let c = &mut kit.colors;
    c.background = tokens.background;
    c.foreground = tokens.foreground;
    c.border = tokens.border;
    c.input = tokens.input;
    c.ring = tokens.ring;
    c.caret = tokens.primary;
    c.primary = tokens.primary;
    c.primary_hover = tokens.primary.opacity(0.9);
    c.primary_active = tokens.primary.opacity(0.8);
    c.primary_foreground = tokens.primary_foreground;
    c.button_primary = tokens.primary;
    c.button_primary_hover = tokens.primary.opacity(0.9);
    c.button_primary_active = tokens.primary.opacity(0.8);
    c.button_primary_foreground = tokens.primary_foreground;
    c.muted = tokens.muted;
    c.muted_foreground = tokens.muted_foreground;
    c.accent = tokens.ghost_pressed();
    c.accent_foreground = tokens.foreground;
    c.popover = tokens.popover;
    c.popover_foreground = tokens.foreground;
    c.selection = tokens.primary.opacity(0.35);
    c.list_active = tokens.primary.opacity(0.14);
    c.list_active_border = tokens.primary.opacity(0.0);
    c.list_hover = tokens.ghost_hover();
    c.danger = tokens.destructive;
    c.switch = tokens.muted;
    c.slider_bar = tokens.primary;
    c.slider_thumb = white();
    c.sidebar = tokens.sidebar;
    c.sidebar_foreground = tokens.sidebar_foreground;
    c.sidebar_border = tokens.sidebar_border;

    cx.set_global(tokens);
    cx.refresh_windows();
}

/// Paint the gradient wash as a small bitmap: dot 0 as the base colour with
/// radial blooms of the others, matching `GradientMesh` in the old app. The
/// image is tiny and gets stretched (and thereby blurred) over the window.
fn render_wash(theme: &GradientTheme) -> Arc<RenderImage> {
    const W: u32 = 192;
    const H: u32 = 128;
    let dots: Vec<Rgba> = theme.dots.iter().map(|d| d.rgb).collect();
    let base = dots.first().copied().unwrap_or(Rgba::hex(0x808080));
    // (colour, centre x, centre y, reach) in unit coordinates.
    let mut blooms: Vec<(Rgba, f32, f32, f32)> = Vec::new();
    if let Some(c) = dots.get(1) {
        blooms.push((*c, 1.0, 1.0, 1.1));
    }
    if let Some(c) = dots.get(2) {
        blooms.push((*c, 1.0, 0.0, 1.1));
        blooms.push((base, 0.0, 0.0, 0.99));
    }
    let mut bytes = Vec::with_capacity((W * H * 4) as usize);
    for y in 0..H {
        for x in 0..W {
            let (u, v) = (x as f32 / (W - 1) as f32, y as f32 / (H - 1) as f32);
            let mut c = base;
            for &(bloom, cx, cy, reach) in &blooms {
                let d = ((u - cx).powi(2) + (v - cy).powi(2)).sqrt() / reach;
                let t = (1.0 - d).clamp(0.0, 1.0);
                let t = t * t * (3.0 - 2.0 * t); // smoothstep falloff
                c = mix(c, bloom, t);
            }
            let ch = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            // RenderImage is BGRA.
            bytes.extend_from_slice(&[ch(c.b), ch(c.g), ch(c.r), 255]);
        }
    }
    let buffer = image::RgbaImage::from_raw(W, H, bytes).expect("wash buffer size");
    let frames: SmallVec<[image::Frame; 1]> = SmallVec::from_elem(image::Frame::new(buffer), 1);
    Arc::new(RenderImage::new(frames))
}

/// The grain tile as translucent grey speckle; `amount` scales its alpha.
fn render_grain(amount: f32) -> Arc<RenderImage> {
    let tile = core_theme::grain_noise_tile();
    let side = (tile.len() as f64).sqrt() as u32;
    let alpha = (amount * 0.18 * 255.0).round() as u32;
    let mut bytes = Vec::with_capacity(tile.len() * 4);
    for g in tile {
        // Premultiplied BGRA: grey speckle at a low, uniform alpha.
        let a = alpha as u8;
        let v = ((g as u32 * alpha) / 255) as u8;
        bytes.extend_from_slice(&[v, v, v, a]);
    }
    let buffer = image::RgbaImage::from_raw(side, side, bytes).expect("grain tile size");
    let frames: SmallVec<[image::Frame; 1]> = SmallVec::from_elem(image::Frame::new(buffer), 1);
    Arc::new(RenderImage::new(frames))
}

fn mix(a: Rgba, b: Rgba, t: f32) -> Rgba {
    Rgba {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: 1.0,
    }
}
