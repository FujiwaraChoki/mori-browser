//! Colour math and the theme model: base light/dark palettes, gradient
//! ("wash") themes per Space, presets, and the derivation of a themed
//! palette from a gradient. No UI types — the UI converts [`Rgba`] itself.
//!
//! Wheel convention: positions are normalised to 0…1 with the centre at
//! (0.5, 0.5); hue 0° (red) sits at 3 o'clock and increases clockwise with
//! screen y pointing down; distance from the centre / 0.5 is saturation.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// An sRGB colour with straight alpha, all components 0…1.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    #[serde(default = "opaque")]
    pub a: f32,
}

fn opaque() -> f32 {
    1.0
}

impl Rgba {
    pub const WHITE: Rgba = Rgba::rgb(1.0, 1.0, 1.0);
    pub const BLACK: Rgba = Rgba::rgb(0.0, 0.0, 0.0);
    pub const TRANSPARENT: Rgba = Rgba {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.0,
    };

    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Rgba {
        Rgba { r, g, b, a }
    }

    /// Opaque colour from components.
    pub const fn rgb(r: f32, g: f32, b: f32) -> Rgba {
        Rgba { r, g, b, a: 1.0 }
    }

    /// Opaque colour from `0xRRGGBB`.
    pub fn hex(rgb: u32) -> Rgba {
        let byte = |shift: u32| ((rgb >> shift) & 0xFF) as f32 / 255.0;
        Rgba::rgb(byte(16), byte(8), byte(0))
    }

    /// Parse `#rgb`, `#rrggbb` or `#rrggbbaa` (the `#` is optional).
    pub fn parse_hex(s: &str) -> Option<Rgba> {
        let s = s.trim().trim_start_matches('#');
        if !s.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        match s.len() {
            3 => {
                let expanded: String = s.chars().flat_map(|c| [c, c]).collect();
                Some(Rgba::hex(u32::from_str_radix(&expanded, 16).ok()?))
            }
            6 => Some(Rgba::hex(u32::from_str_radix(s, 16).ok()?)),
            8 => {
                let v = u32::from_str_radix(s, 16).ok()?;
                Some(Rgba::hex(v >> 8).with_alpha((v & 0xFF) as f32 / 255.0))
            }
            _ => None,
        }
    }

    /// `0xRRGGBB` (alpha dropped, components clamped and rounded).
    pub fn to_u32(self) -> u32 {
        let byte = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u32;
        (byte(self.r) << 16) | (byte(self.g) << 8) | byte(self.b)
    }

    /// `#RRGGBB` (alpha dropped).
    pub fn to_hex_string(self) -> String {
        format!("#{:06X}", self.to_u32())
    }

    pub fn with_alpha(self, a: f32) -> Rgba {
        Rgba { a, ..self }
    }

    /// CSS Color 4 `oklch(L C h)` → OKLab → linear sRGB → gamma sRGB, with
    /// each linear channel clamped to 0…1 (matches the original OKLCH.swift).
    pub fn oklch(l: f32, c: f32, h: f32) -> Rgba {
        let (l, c, h) = (l as f64, c as f64, h as f64);
        let hr = h.to_radians();
        let a = c * hr.cos();
        let bb = c * hr.sin();

        let l_ = l + 0.3963377774 * a + 0.2158037573 * bb;
        let m_ = l - 0.1055613458 * a - 0.0638541728 * bb;
        let s_ = l - 0.0894841775 * a - 1.2914855480 * bb;
        let (l3, m3, s3) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);

        let r = 4.0767416621 * l3 - 3.3077115913 * m3 + 0.2309699292 * s3;
        let g = -1.2684380046 * l3 + 2.6097574011 * m3 - 0.3413193965 * s3;
        let b = -0.0041960863 * l3 - 0.7034186147 * m3 + 1.7076147010 * s3;

        fn gamma(c: f64) -> f32 {
            let c = c.clamp(0.0, 1.0);
            (if c <= 0.0031308 {
                12.92 * c
            } else {
                1.055 * c.powf(1.0 / 2.4) - 0.055
            }) as f32
        }
        Rgba::rgb(gamma(r), gamma(g), gamma(b))
    }

    /// `(hue°, saturation, lightness)` with hue in 0…360 and s/l in 0…1.
    pub fn to_hsl(self) -> (f32, f32, f32) {
        let (r, g, b) = (self.r as f64, self.g as f64, self.b as f64);
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let delta = max - min;
        let l = (max + min) / 2.0;
        if delta <= 0.0 {
            return (0.0, 0.0, l as f32);
        }
        let s = delta / (1.0 - (2.0 * l - 1.0).abs());
        let mut h = if max == r {
            60.0 * (((g - b) / delta) % 6.0)
        } else if max == g {
            60.0 * ((b - r) / delta + 2.0)
        } else {
            60.0 * ((r - g) / delta + 4.0)
        };
        if h < 0.0 {
            h += 360.0;
        }
        (h as f32, s as f32, l as f32)
    }

    /// Opaque colour from HSL (hue in degrees, wrapped; s/l in 0…1).
    pub fn from_hsl(h: f32, s: f32, l: f32) -> Rgba {
        let (h, s, l) = (h as f64, s as f64, l as f64);
        let hue = h.rem_euclid(360.0);
        let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
        let x = c * (1.0 - ((hue / 60.0) % 2.0 - 1.0).abs());
        let m = l - c / 2.0;
        let (r1, g1, b1) = match hue {
            h if h < 60.0 => (c, x, 0.0),
            h if h < 120.0 => (x, c, 0.0),
            h if h < 180.0 => (0.0, c, x),
            h if h < 240.0 => (0.0, x, c),
            h if h < 300.0 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        Rgba::rgb((r1 + m) as f32, (g1 + m) as f32, (b1 + m) as f32)
    }

    /// WCAG relative luminance (0 = black, 1 = white).
    pub fn luminance(self) -> f32 {
        fn lin(c: f32) -> f32 {
            if c <= 0.03928 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        }
        0.2126 * lin(self.r) + 0.7152 * lin(self.g) + 0.0722 * lin(self.b)
    }

    /// Linear interpolation towards `other` (`t` = 0 → self, 1 → other).
    pub fn mix(self, other: Rgba, t: f32) -> Rgba {
        let t = t.clamp(0.0, 1.0);
        let lerp = |a: f32, b: f32| a + (b - a) * t;
        Rgba::new(
            lerp(self.r, other.r),
            lerp(self.g, other.g),
            lerp(self.b, other.b),
            lerp(self.a, other.a),
        )
    }
}

// ---------------------------------------------------------------------------
// Palettes
// ---------------------------------------------------------------------------

/// The full set of colour tokens for one appearance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub background: Rgba,
    pub foreground: Rgba,
    pub card: Rgba,
    pub popover: Rgba,
    pub primary: Rgba,
    pub primary_foreground: Rgba,
    pub secondary: Rgba,
    pub secondary_foreground: Rgba,
    pub muted: Rgba,
    pub muted_foreground: Rgba,
    pub accent: Rgba,
    pub accent_foreground: Rgba,
    pub destructive: Rgba,
    pub border: Rgba,
    pub input: Rgba,
    pub ring: Rgba,
    pub sidebar: Rgba,
    pub sidebar_foreground: Rgba,
    pub sidebar_primary: Rgba,
    pub sidebar_accent: Rgba,
    pub sidebar_border: Rgba,
    pub status_info: Rgba,
    pub status_success: Rgba,
    pub status_warning: Rgba,
}

impl Palette {
    /// The light base palette.
    pub fn light() -> Palette {
        let fg = Rgba::oklch(0.165, 0.018, 248.5103);
        let primary = Rgba::oklch(0.645, 0.11, 241.2);
        Palette {
            background: Rgba::hex(0xF7F7F7),
            foreground: fg,
            card: Rgba::oklch(0.985, 0.0015, 220.0),
            popover: Rgba::oklch(0.998, 0.0008, 240.0),
            primary,
            primary_foreground: Rgba::oklch(1.0, 0.0, 0.0),
            secondary: fg,
            secondary_foreground: Rgba::oklch(1.0, 0.0, 0.0),
            muted: Rgba::oklch(0.935, 0.002, 245.0),
            muted_foreground: Rgba::oklch(0.48, 0.012, 248.5103),
            accent: Rgba::hex(0xEDEDED),
            accent_foreground: primary,
            destructive: Rgba::oklch(0.635, 0.24, 28.0),
            border: Rgba::oklch(0.92, 0.0, 0.0),
            input: Rgba::hex(0xEDEDED),
            ring: Rgba::oklch(0.55, 0.0, 0.0),
            sidebar: Rgba::hex(0xEBEBEB),
            sidebar_foreground: fg,
            sidebar_primary: primary,
            sidebar_accent: Rgba::hex(0xFFFFFF),
            sidebar_border: Rgba::oklch(0.915, 0.0, 0.0),
            status_info: Rgba::oklch(0.5, 0.134, 242.749),
            status_success: Rgba::oklch(0.527, 0.154, 150.069),
            status_warning: Rgba::oklch(0.555, 0.163, 48.998),
        }
    }

    /// The dark base palette (neutral chrome is chroma-0).
    pub fn dark() -> Palette {
        let fg = Rgba::hex(0xE8EAED);
        let primary = Rgba::oklch(0.62, 0.13, 241.5);
        Palette {
            background: Rgba::hex(0x222222),
            foreground: fg,
            card: Rgba::oklch(0.36, 0.0, 0.0),
            popover: Rgba::oklch(0.3, 0.0, 0.0),
            primary,
            primary_foreground: Rgba::oklch(0.28, 0.008, 235.0),
            secondary: fg,
            secondary_foreground: Rgba::oklch(0.3, 0.0, 0.0),
            muted: Rgba::oklch(0.36, 0.0, 0.0),
            muted_foreground: Rgba::hex(0xAEB6BF),
            accent: Rgba::oklch(0.42, 0.0, 0.0),
            accent_foreground: primary,
            destructive: Rgba::oklch(0.62, 0.22, 27.0),
            border: Rgba::oklch(0.45, 0.0, 0.0),
            input: Rgba::oklch(0.4, 0.02, 240.0),
            ring: Rgba::oklch(0.6, 0.0, 0.0),
            sidebar: Rgba::hex(0x151515),
            sidebar_foreground: Rgba::hex(0xF1F3F5),
            sidebar_primary: Rgba::oklch(0.6, 0.12, 241.0),
            sidebar_accent: Rgba::hex(0x2A2A2A),
            sidebar_border: Rgba::oklch(0.48, 0.0, 0.0),
            status_info: Rgba::oklch(0.746, 0.16, 232.661),
            status_success: Rgba::oklch(0.792, 0.209, 151.711),
            status_warning: Rgba::oklch(0.828, 0.189, 84.429),
        }
    }

    /// The base palette for a scheme.
    pub fn for_scheme(dark: bool) -> Palette {
        if dark {
            Palette::dark()
        } else {
            Palette::light()
        }
    }
}

// ---------------------------------------------------------------------------
// Gradient themes
// ---------------------------------------------------------------------------

/// Explicit light/dark override carried by a gradient theme.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SchemeOverride {
    #[default]
    Auto,
    Light,
    Dark,
}

impl SchemeOverride {
    pub const ALL: [SchemeOverride; 3] = [
        SchemeOverride::Auto,
        SchemeOverride::Light,
        SchemeOverride::Dark,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SchemeOverride::Auto => "Auto",
            SchemeOverride::Light => "Light",
            SchemeOverride::Dark => "Dark",
        }
    }
}

/// One colour dot on the circular picker.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GradientDot {
    pub id: Uuid,
    pub rgb: Rgba,
    /// Normalised wheel position (centre 0.5, 0.5).
    pub x: f32,
    pub y: f32,
    /// Explicit HSL lightness 0…100; `None` = 50.
    #[serde(default)]
    pub lightness: Option<f32>,
    #[serde(default)]
    pub is_primary: bool,
}

impl GradientDot {
    /// A dot for a known colour, placed on the wheel from its hue/saturation.
    pub fn from_rgb(rgb: Rgba, is_primary: bool) -> GradientDot {
        let rgb = rgb.with_alpha(1.0);
        let (x, y, lightness) = wheel_placement(rgb);
        GradientDot {
            id: Uuid::new_v4(),
            rgb,
            x,
            y,
            lightness: Some(lightness),
            is_primary,
        }
    }

    /// A dot dragged to wheel position (`x`, `y`).
    pub fn from_wheel(x: f32, y: f32, lightness: Option<f32>, is_primary: bool) -> GradientDot {
        GradientDot {
            id: Uuid::new_v4(),
            rgb: dot_from_wheel(x, y, lightness),
            x,
            y,
            lightness,
            is_primary,
        }
    }
}

/// A Space's chrome theme: 0…3 dots washed behind the chrome.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GradientTheme {
    #[serde(default)]
    pub dots: Vec<GradientDot>,
    /// Wash strength, see [`GradientTheme::MIN_OPACITY`]…[`GradientTheme::MAX_OPACITY`].
    #[serde(default = "half")]
    pub opacity: f32,
    /// Grain amount 0…1.
    #[serde(default)]
    pub texture: f32,
    #[serde(default)]
    pub scheme_override: SchemeOverride,
    /// The preset this came from (or `solid:#RRGGBB`), if any.
    #[serde(default)]
    pub preset_id: Option<String>,
}

fn half() -> f32 {
    0.5
}

impl Default for GradientTheme {
    fn default() -> Self {
        GradientTheme::none()
    }
}

impl GradientTheme {
    pub const MIN_OPACITY: f32 = 0.3;
    pub const MAX_OPACITY: f32 = 0.9;
    pub const MAX_DOTS: usize = 3;
    /// `preset_id` prefix marking a solid-colour theme.
    pub const SOLID_PREFIX: &'static str = "solid:";

    /// "No theme": the plain palette tint.
    pub fn none() -> GradientTheme {
        GradientTheme {
            dots: Vec::new(),
            opacity: 0.5,
            texture: 0.0,
            scheme_override: SchemeOverride::Auto,
            preset_id: None,
        }
    }

    /// A flat single-colour wash (opacity 0.6, auto scheme).
    pub fn solid(rgb: Rgba) -> GradientTheme {
        let dot = GradientDot::from_rgb(rgb, true);
        let hex = dot.rgb.to_hex_string();
        GradientTheme {
            dots: vec![dot],
            opacity: 0.6,
            texture: 0.0,
            scheme_override: SchemeOverride::Auto,
            preset_id: Some(format!("{}{hex}", Self::SOLID_PREFIX)),
        }
    }

    pub fn is_none(&self) -> bool {
        self.dots.is_empty()
    }

    /// `#RRGGBB` when this is a solid theme.
    pub fn solid_hex(&self) -> Option<&str> {
        self.preset_id.as_deref()?.strip_prefix(Self::SOLID_PREFIX)
    }

    /// The dot driving the accent: the flagged primary, else the first.
    pub fn primary_dot(&self) -> Option<&GradientDot> {
        self.dots
            .iter()
            .find(|d| d.is_primary)
            .or_else(|| self.dots.first())
    }

    pub fn clamped_opacity(&self) -> f32 {
        if self.opacity.is_nan() {
            return 0.5;
        }
        self.opacity.clamp(Self::MIN_OPACITY, Self::MAX_OPACITY)
    }

    pub fn clamped_texture(&self) -> f32 {
        if self.texture.is_nan() {
            0.0
        } else {
            self.texture.clamp(0.0, 1.0)
        }
    }

    /// Dot colours in wash order (dot[0] is the base).
    pub fn colors(&self) -> Vec<Rgba> {
        self.dots.iter().map(|d| d.rgb).collect()
    }

    /// The component-wise average dot colour.
    pub fn average_color(&self) -> Option<Rgba> {
        if self.dots.is_empty() {
            return None;
        }
        let n = self.dots.len() as f32;
        let (r, g, b) = self.dots.iter().fold((0.0, 0.0, 0.0), |(r, g, b), d| {
            (r + d.rgb.r, g + d.rgb.g, b + d.rgb.b)
        });
        Some(Rgba::rgb(r / n, g / n, b / n))
    }
}

/// How secondary dots derive from the primary's hue.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Harmony {
    #[default]
    Floating,
    Complementary,
    SingleAnalogous,
    SplitComplementary,
    Analogous,
    Triadic,
}

impl Harmony {
    pub const ALL: [Harmony; 6] = [
        Harmony::Floating,
        Harmony::Complementary,
        Harmony::SingleAnalogous,
        Harmony::SplitComplementary,
        Harmony::Analogous,
        Harmony::Triadic,
    ];

    /// Hue offsets (degrees) of the generated secondary dots.
    pub fn offsets(self) -> &'static [f32] {
        match self {
            Harmony::Floating => &[],
            Harmony::Complementary => &[180.0],
            Harmony::SingleAnalogous => &[310.0],
            Harmony::SplitComplementary => &[150.0, 210.0],
            Harmony::Analogous => &[50.0, 310.0],
            Harmony::Triadic => &[120.0, 240.0],
        }
    }

    /// Primary + secondaries.
    pub fn dot_count(self) -> usize {
        self.offsets().len() + 1
    }

    pub fn label(self) -> &'static str {
        match self {
            Harmony::Floating => "Free",
            Harmony::Complementary => "Complementary",
            Harmony::SingleAnalogous => "Analogous",
            Harmony::SplitComplementary => "Split",
            Harmony::Analogous => "Analogous ×2",
            Harmony::Triadic => "Triadic",
        }
    }
}

/// Secondary dots for `harmony`: same saturation/lightness, hue rotated.
pub fn harmony_dots(primary: &GradientDot, harmony: Harmony) -> Vec<GradientDot> {
    let (h, s, l) = primary.rgb.to_hsl();
    harmony
        .offsets()
        .iter()
        .map(|offset| GradientDot::from_rgb(Rgba::from_hsl(h + offset, s, l), false))
        .collect()
}

/// Colour at a normalised wheel point: radius → saturation, angle → hue,
/// lightness (0…100, default 50) → HSL lightness.
pub fn dot_from_wheel(x: f32, y: f32, lightness: Option<f32>) -> Rgba {
    let dx = x - 0.5;
    let dy = y - 0.5;
    let saturation = (dx.hypot(dy) / 0.5).min(1.0);
    let mut angle = dy.atan2(dx).to_degrees();
    if angle < 0.0 {
        angle += 360.0;
    }
    Rgba::from_hsl(angle, saturation, lightness.unwrap_or(50.0) / 100.0)
}

/// Where a colour sits on the wheel (inverse of [`dot_from_wheel`]).
pub fn wheel_position(rgb: Rgba) -> (f32, f32) {
    let (x, y, _) = wheel_placement(rgb);
    (x, y)
}

/// Wheel position plus HSL lightness (0…100) for a colour.
pub fn wheel_placement(rgb: Rgba) -> (f32, f32, f32) {
    let (h, s, l) = rgb.to_hsl();
    let rad = h.to_radians();
    let radius = s.min(1.0) * 0.5;
    (
        0.5 + rad.cos() * radius,
        0.5 + rad.sin() * radius,
        l * 100.0,
    )
}

// ---------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------

/// Luminance below which a colour counts as dark.
pub const DARK_LUMINANCE: f32 = 0.4;

pub fn is_dark(c: Rgba) -> bool {
    c.luminance() < DARK_LUMINANCE
}

/// A legible text colour over `c`: near-white on dark, near-black otherwise.
pub fn contrasting_text(c: Rgba) -> Rgba {
    if is_dark(c) {
        Rgba::rgb(0.96, 0.96, 0.96)
    } else {
        Rgba::rgb(0.12, 0.12, 0.13)
    }
}

/// The UI accent derived from the theme's primary dot, nudged for legibility
/// in the given scheme. `None` when the theme has no dots.
pub fn accent_for(theme: &GradientTheme, dark: bool) -> Option<Rgba> {
    let (h, s, l) = theme.primary_dot()?.rgb.to_hsl();
    let (s, l) = if dark {
        ((s * 1.15).min(1.0), l.max(0.6))
    } else {
        ((s * 1.08).min(1.0), l.clamp(0.4, 0.55))
    };
    Some(Rgba::from_hsl(h, s, l))
}

/// Whether the chrome renders dark: the theme's override wins; `Auto` uses
/// the average dot luminance (< 0.4 → dark); no dots → the appearance setting.
pub fn effective_dark(theme: &GradientTheme, appearance_dark: bool) -> bool {
    match theme.scheme_override {
        SchemeOverride::Light => false,
        SchemeOverride::Dark => true,
        SchemeOverride::Auto if theme.dots.is_empty() => appearance_dark,
        SchemeOverride::Auto => {
            let avg =
                theme.dots.iter().map(|d| d.rgb.luminance()).sum::<f32>() / theme.dots.len() as f32;
            avg < DARK_LUMINANCE
        }
    }
}

/// Foreground / muted / sidebar foregrounds tuned per preset.
fn preset_foregrounds(preset_id: &str) -> Option<(u32, u32, u32)> {
    Some(match preset_id {
        "evangelion" => (0xF4EEFF, 0xCEC4E8, 0xFBF7FF),
        "tokyo-ghoul" => (0xFFF0F2, 0xD9AEB5, 0xFFF6F6),
        "demon-slayer" => (0xEAF8F2, 0xBBD9D0, 0xF3FFF9),
        "jujutsu-kaisen" => (0xEAF6FF, 0xB8CCE6, 0xF4FAFF),
        "chainsaw-man" => (0xFFF1E8, 0xDDB8A7, 0xFFF7F1),
        "your-name" => (0xFFF2EA, 0xDCC2D2, 0xFFF8EF),
        "sailor-moon" => (0x232746, 0x657095, 0x1E2441),
        _ => return None,
    })
}

/// Layer a gradient theme over a base palette. Only brand tokens (primary,
/// its foreground, ring, accent foreground, sidebar primary) take the derived
/// accent, and the text foregrounds follow the preset table or the wash's
/// average colour. Surfaces keep the base values. No theme → `base`.
pub fn themed_palette(base: Palette, theme: &GradientTheme, dark: bool) -> Palette {
    let Some(accent) = accent_for(theme, dark) else {
        return base;
    };
    let mut p = base;
    p.primary = accent;
    p.primary_foreground = contrasting_text(accent);
    p.ring = accent;
    p.accent_foreground = accent;
    p.sidebar_primary = accent;

    let (fg, muted, sidebar) = match theme.preset_id.as_deref().and_then(preset_foregrounds) {
        Some((fg, muted, sidebar)) => (Rgba::hex(fg), Rgba::hex(muted), Rgba::hex(sidebar)),
        None => {
            // `accent_for` succeeded, so there is at least one dot.
            let avg = theme.average_color().unwrap_or(Rgba::BLACK);
            if is_dark(contrasting_text(avg)) {
                (
                    Rgba::hex(0x22263D),
                    Rgba::hex(0x626C86),
                    Rgba::hex(0x1F253A),
                )
            } else {
                (
                    Rgba::hex(0xF5F7FA),
                    Rgba::hex(0xC4CDD8),
                    Rgba::hex(0xFFFFFF),
                )
            }
        }
    };
    p.foreground = fg;
    p.secondary = fg;
    p.muted_foreground = muted;
    p.sidebar_foreground = sidebar;
    p
}

/// The 128×128 monochrome grain tile (one grey byte per pixel, row-major),
/// from the same LCG as the original so the texture looks identical.
pub fn grain_noise_tile() -> Vec<u8> {
    const SIZE: usize = 128;
    let mut state: u64 = 0x4D4F_5249_5F47_524E;
    (0..SIZE * SIZE)
        .map(|_| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 56) as u8
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Presets
// ---------------------------------------------------------------------------

/// A curated gradient theme.
#[derive(Clone, Debug, PartialEq)]
pub struct Preset {
    pub id: &'static str,
    pub name: &'static str,
    pub subtitle: &'static str,
    /// `0xRRGGBB`, primary first.
    pub colors: [u32; 3],
    pub opacity: f32,
    pub texture: f32,
    pub scheme: SchemeOverride,
}

impl Preset {
    /// The theme this preset produces (fresh dot ids each call).
    pub fn theme(&self) -> GradientTheme {
        GradientTheme {
            dots: self
                .colors
                .iter()
                .enumerate()
                .map(|(i, &c)| GradientDot::from_rgb(Rgba::hex(c), i == 0))
                .collect(),
            opacity: self.opacity,
            texture: self.texture,
            scheme_override: self.scheme,
            preset_id: Some(self.id.to_string()),
        }
    }
}

/// The preset lineup, in gallery order.
pub fn presets() -> Vec<Preset> {
    use SchemeOverride::{Dark, Light};
    let p = |id, name, subtitle, colors, opacity, texture, scheme| Preset {
        id,
        name,
        subtitle,
        colors,
        opacity,
        texture,
        scheme,
    };
    vec![
        p(
            "evangelion",
            "Evangelion",
            "Unit-01",
            [0x765898, 0x52D053, 0xE6770B],
            0.58,
            0.35,
            Dark,
        ),
        p(
            "tokyo-ghoul",
            "Tokyo Ghoul",
            "Kakugan",
            [0xD11A1F, 0x4A0A12, 0x0F0A0B],
            0.62,
            0.55,
            Dark,
        ),
        p(
            "demon-slayer",
            "Demon Slayer",
            "Checkered Haori",
            [0x4EB18D, 0x211F20, 0x58C29E],
            0.55,
            0.30,
            Dark,
        ),
        p(
            "jujutsu-kaisen",
            "Jujutsu Kaisen",
            "Cursed Energy",
            [0x2BA3E8, 0x14182E, 0x6A3FA0],
            0.56,
            0.32,
            Dark,
        ),
        p(
            "chainsaw-man",
            "Chainsaw Man",
            "Pochita",
            [0xB52C2F, 0x1B1513, 0xE07B3A],
            0.60,
            0.55,
            Dark,
        ),
        p(
            "your-name",
            "Your Name",
            "Kataware-doki",
            [0xF4A65B, 0xC95B7E, 0x2A2D5E],
            0.50,
            0.22,
            Dark,
        ),
        p(
            "sailor-moon",
            "Sailor Moon",
            "Moonlight",
            [0xFB87B0, 0x5C79CE, 0xFBD15B],
            0.46,
            0.15,
            Light,
        ),
        p(
            "sunset",
            "Sunset",
            "Warm horizon",
            [0xF08A5D, 0xB84A62, 0x355C7D],
            0.52,
            0.18,
            Dark,
        ),
        p(
            "forest",
            "Forest",
            "Moss canopy",
            [0x2F7D5C, 0x173F35, 0xA7C957],
            0.54,
            0.24,
            Dark,
        ),
        p(
            "nordic",
            "Nordic",
            "Fjord light",
            [0x88C0D0, 0x5E81AC, 0xECEFF4],
            0.48,
            0.12,
            Light,
        ),
        p(
            "ocean",
            "Ocean",
            "Blue current",
            [0x0077B6, 0x00B4D8, 0x023E8A],
            0.52,
            0.20,
            Dark,
        ),
        p(
            "lavender",
            "Lavender",
            "Quiet bloom",
            [0xB8A1FF, 0xF1C6E7, 0x7C6CB0],
            0.44,
            0.12,
            Light,
        ),
    ]
}

/// Solid-colour swatches: (name, `0xRRGGBB`).
pub const SOLID_SWATCHES: [(&str, u32); 10] = [
    ("Violet", 0x6E56CF),
    ("Blue", 0x3B82F6),
    ("Teal", 0x0EA5A4),
    ("Green", 0x22C55E),
    ("Amber", 0xEAB308),
    ("Orange", 0xF97316),
    ("Red", 0xEF4444),
    ("Pink", 0xEC4899),
    ("Slate", 0x64748B),
    ("Graphite", 0x1F2937),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Rgba, hex: u32) -> bool {
        let b = Rgba::hex(hex);
        let tol = 2.0 / 255.0 + 1e-4;
        (a.r - b.r).abs() <= tol && (a.g - b.g).abs() <= tol && (a.b - b.b).abs() <= tol
    }

    #[test]
    fn oklch_matches_reference_hex() {
        let cases = [
            ((0.645, 0.11, 241.2), 0x4895CA),
            ((0.62, 0.13, 241.5), 0x298ECD),
            ((0.165, 0.018, 248.51), 0x080F16),
            ((0.36, 0.0, 0.0), 0x3D3D3D),
            ((0.635, 0.24, 28.0), 0xFB2A27),
            ((0.5, 0.134, 242.7), 0x0069A8),
            ((0.828, 0.189, 84.4), 0xFFB900),
            ((0.92, 0.0, 0.0), 0xE4E4E4),
        ];
        for ((l, c, h), hex) in cases {
            let got = Rgba::oklch(l, c, h);
            assert!(
                close(got, hex),
                "oklch({l} {c} {h}) = {} ≠ #{hex:06X}",
                got.to_hex_string()
            );
        }
        assert_eq!(Rgba::oklch(1.0, 0.0, 0.0).to_u32(), 0xFFFFFF);
    }

    #[test]
    fn hex_and_hsl_roundtrip() {
        assert_eq!(Rgba::hex(0x4895CA).to_hex_string(), "#4895CA");
        assert_eq!(Rgba::parse_hex("#abc").unwrap().to_u32(), 0xAABBCC);
        assert_eq!(Rgba::parse_hex("11223380").unwrap().a, 128.0 / 255.0);
        assert!(Rgba::parse_hex("#xyz").is_none());
        for hex in [
            0x765898, 0x52D053, 0xE6770B, 0x000000, 0xFFFFFF, 0x808080, 0xFF0000,
        ] {
            let c = Rgba::hex(hex);
            let (h, s, l) = c.to_hsl();
            assert_eq!(Rgba::from_hsl(h, s, l).to_u32(), hex);
        }
        let (h, s, l) = Rgba::hex(0x00FF00).to_hsl();
        assert!((h - 120.0).abs() < 1e-3 && (s - 1.0).abs() < 1e-5 && (l - 0.5).abs() < 1e-5);
    }

    #[test]
    fn luminance_and_contrast() {
        assert!((Rgba::WHITE.luminance() - 1.0).abs() < 1e-5);
        assert_eq!(Rgba::BLACK.luminance(), 0.0);
        assert_eq!(contrasting_text(Rgba::BLACK), Rgba::rgb(0.96, 0.96, 0.96));
        assert_eq!(contrasting_text(Rgba::WHITE), Rgba::rgb(0.12, 0.12, 0.13));
        let mid = Rgba::BLACK.mix(Rgba::WHITE, 0.5);
        assert!((mid.r - 0.5).abs() < 1e-6);
    }

    #[test]
    fn wheel_roundtrip() {
        // 3 o'clock at the rim is fully saturated red.
        assert_eq!(dot_from_wheel(1.0, 0.5, None).to_u32(), 0xFF0000);
        // Screen-down (6 o'clock) is hue 90°.
        let (h, _, _) = dot_from_wheel(0.5, 1.0, None).to_hsl();
        assert!((h - 90.0).abs() < 1e-3);
        assert_eq!(dot_from_wheel(0.5, 0.5, Some(100.0)).to_u32(), 0xFFFFFF);
        let c = Rgba::hex(0x3B82F6);
        let (x, y, l) = wheel_placement(c);
        assert_eq!(dot_from_wheel(x, y, Some(l)).to_u32(), 0x3B82F6);
        assert_eq!(wheel_position(c), (x, y));
    }

    #[test]
    fn accent_derivation() {
        let theme = GradientTheme::solid(Rgba::hex(0x1F2937));
        let (h0, s0, _) = Rgba::hex(0x1F2937).to_hsl();
        let (h, s, l) = accent_for(&theme, true).unwrap().to_hsl();
        assert!((h - h0).abs() < 0.5);
        assert!((s - (s0 * 1.15).min(1.0)).abs() < 0.01);
        assert!((l - 0.6).abs() < 0.01);
        let (_, s, l) = accent_for(&theme, false).unwrap().to_hsl();
        assert!((s - (s0 * 1.08).min(1.0)).abs() < 0.01);
        assert!((l - 0.4).abs() < 0.01);
        assert!(accent_for(&GradientTheme::none(), true).is_none());

        // Primary flag wins over order.
        let mut t = GradientTheme::none();
        t.dots = vec![
            GradientDot::from_rgb(Rgba::hex(0xFF0000), false),
            GradientDot::from_rgb(Rgba::hex(0x0000FF), true),
        ];
        let (h, _, _) = accent_for(&t, false).unwrap().to_hsl();
        assert!((h - 240.0).abs() < 0.5);
    }

    #[test]
    fn scheme_resolution() {
        let none = GradientTheme::none();
        assert!(effective_dark(&none, true));
        assert!(!effective_dark(&none, false));
        assert!(effective_dark(
            &GradientTheme::solid(Rgba::hex(0x1F2937)),
            false
        ));
        assert!(!effective_dark(
            &GradientTheme::solid(Rgba::hex(0xECEFF4)),
            true
        ));
        let mut t = GradientTheme::solid(Rgba::hex(0xECEFF4));
        t.scheme_override = SchemeOverride::Dark;
        assert!(effective_dark(&t, false));
    }

    #[test]
    fn themed_palettes() {
        let base = Palette::dark();
        assert_eq!(themed_palette(base, &GradientTheme::none(), true), base);

        let eva = presets()[0].theme();
        let p = themed_palette(base, &eva, true);
        assert_eq!(p.foreground.to_u32(), 0xF4EEFF);
        assert_eq!(p.muted_foreground.to_u32(), 0xCEC4E8);
        assert_eq!(p.sidebar_foreground.to_u32(), 0xFBF7FF);
        assert_eq!(p.primary, accent_for(&eva, true).unwrap());
        assert_eq!(p.ring, p.primary);
        assert_eq!(p.sidebar_primary, p.primary);
        assert_eq!(p.card, base.card, "surfaces keep base values");

        let light = themed_palette(
            Palette::light(),
            &GradientTheme::solid(Rgba::hex(0xECEFF4)),
            false,
        );
        assert_eq!(light.foreground.to_u32(), 0x22263D);
        let dark = themed_palette(base, &GradientTheme::solid(Rgba::hex(0x1F2937)), true);
        assert_eq!(dark.foreground.to_u32(), 0xF5F7FA);
        assert_eq!(dark.sidebar_foreground.to_u32(), 0xFFFFFF);
    }

    #[test]
    fn presets_and_solids() {
        let all = presets();
        assert_eq!(all.len(), 12);
        let t = all[6].theme();
        assert_eq!(t.preset_id.as_deref(), Some("sailor-moon"));
        assert_eq!(t.scheme_override, SchemeOverride::Light);
        assert!(t.dots[0].is_primary && !t.dots[1].is_primary);
        assert_eq!(t.dots[1].rgb.to_u32(), 0x5C79CE);

        let s = GradientTheme::solid(Rgba::hex(SOLID_SWATCHES[0].1));
        assert_eq!(s.solid_hex(), Some("#6E56CF"));
        assert_eq!(s.opacity, 0.6);
        assert!(!s.is_none() && GradientTheme::none().is_none());

        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<GradientTheme>(&json).unwrap(), s);
        assert_eq!(
            serde_json::from_str::<GradientTheme>("{}").unwrap(),
            GradientTheme::none()
        );
    }

    #[test]
    fn harmonies() {
        let primary = GradientDot::from_rgb(Rgba::hex(0xFF0000), true);
        let dots = harmony_dots(&primary, Harmony::Triadic);
        assert_eq!(dots.len(), 2);
        assert_eq!(dots[0].rgb.to_u32(), 0x00FF00);
        assert_eq!(dots[1].rgb.to_u32(), 0x0000FF);
        assert!(harmony_dots(&primary, Harmony::Floating).is_empty());
        assert_eq!(Harmony::SplitComplementary.dot_count(), 3);
    }

    #[test]
    fn grain_is_deterministic() {
        let a = grain_noise_tile();
        assert_eq!(a.len(), 128 * 128);
        assert_eq!(a, grain_noise_tile());
    }
}
