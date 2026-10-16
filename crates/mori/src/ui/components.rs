//! Small shared pieces of Mori's chrome: icons, ghost buttons, favicons,
//! key caps.

use std::sync::Arc;

use gpui_kit::*;

use crate::theme::{ActiveTokens, metrics};

/// A Lucide icon by name (see gpui-kit-assets), tinted with `color`.
pub fn icon(name: &str, size: Pixels, color: Hsla) -> Svg {
    svg()
        .path(SharedString::from(format!("icons/{name}.svg")))
        .size(size)
        .flex_none()
        .text_color(color)
}

/// A square, borderless icon button with Mori's ghost hover/press fill.
pub fn icon_button(
    id: impl Into<ElementId>,
    icon_name: &str,
    size: Pixels,
    enabled: bool,
    cx: &App,
) -> Stateful<Div> {
    let t = cx.tokens();
    let fg = if enabled {
        t.sidebar_foreground.opacity(0.85)
    } else {
        t.sidebar_foreground.opacity(0.35)
    };
    let mut el = div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(size)
        .rounded(metrics::RADIUS)
        .child(icon(icon_name, (size * 0.57).round(), fg));
    if enabled {
        el = el
            .cursor_pointer()
            .hover(|s| s.bg(t.ghost_hover()))
            .active(|s| s.bg(t.ghost_pressed()));
    }
    el
}

/// A tab's favicon, or a globe when it has none.
pub fn favicon(image: Option<Arc<RenderImage>>, size: Pixels, dimmed: bool, cx: &App) -> AnyElement {
    let t = cx.tokens();
    match image {
        Some(image) => img(image)
            .size(size)
            .flex_none()
            .rounded(px(3.))
            .opacity(if dimmed { 0.5 } else { 1.0 })
            .into_any_element(),
        None => icon("globe", size, t.sidebar_foreground.opacity(if dimmed { 0.35 } else { 0.6 }))
            .into_any_element(),
    }
}

/// A keyboard shortcut rendered as key caps.
pub fn shortcut_caps(keys: &str, cx: &App) -> Div {
    let t = cx.tokens();
    let label = crate::actions::shortcut_label(keys);
    h_flex().gap_0p5().children(label.split('+').map(|k| {
        div()
            .min_w(px(20.))
            .h(px(20.))
            .px_1()
            .flex()
            .items_center()
            .justify_center()
            .rounded(metrics::RADIUS_MD)
            .bg(t.foreground.opacity(0.07))
            .text_size(metrics::TEXT_CAPTION)
            .text_color(t.muted_foreground)
            .child(k.to_string())
    }))
}

pub fn h_flex() -> Div {
    div().flex().flex_row().items_center()
}

pub fn v_flex() -> Div {
    div().flex().flex_col()
}

/// A card surface used by popovers and dialogs.
pub fn popover_surface(cx: &App) -> Div {
    let t = cx.tokens();
    div()
        .bg(t.popover)
        .rounded(metrics::RADIUS_POPOVER)
        .border_1()
        .border_color(t.border.opacity(0.6))
        .shadow(t.popover_shadow())
        .text_color(t.foreground)
        .text_size(metrics::TEXT_BASE)
}

/// A labelled button used in dialogs and panels.
pub fn text_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    primary: bool,
    cx: &App,
) -> Stateful<Div> {
    let t = cx.tokens();
    let (bg, fg) = if primary {
        (t.primary, t.primary_foreground)
    } else {
        (t.foreground.opacity(0.07), t.foreground)
    };
    div()
        .id(id)
        .h(px(30.))
        .px_3()
        .flex()
        .items_center()
        .justify_center()
        .rounded(metrics::RADIUS_MD)
        .bg(bg)
        .text_color(fg)
        .text_size(metrics::TEXT_BASE)
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .hover(move |s| s.bg(bg.opacity(if primary { 0.9 } else { 0.12 })))
        .child(label.into())
}

/// Format a byte count like "1.4 MB".
pub fn format_bytes(bytes: i64) -> String {
    let b = bytes.max(0) as f64;
    match b {
        b if b >= 1e9 => format!("{:.1} GB", b / 1e9),
        b if b >= 1e6 => format!("{:.1} MB", b / 1e6),
        b if b >= 1e3 => format!("{:.0} KB", b / 1e3),
        b => format!("{b:.0} B"),
    }
}

/// "3m ago"-style relative time.
pub fn relative_time(then: f64, now: f64) -> String {
    let d = (now - then).max(0.0);
    match d {
        d if d < 60.0 => "just now".into(),
        d if d < 3600.0 => format!("{}m ago", (d / 60.0) as u64),
        d if d < 86_400.0 => format!("{}h ago", (d / 3600.0) as u64),
        d => format!("{}d ago", (d / 86_400.0) as u64),
    }
}
