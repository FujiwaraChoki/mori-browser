//! Capture (Arc's screenshot tool): the geometry of a dragged region, pixel
//! cropping, decoding DevTools' base64 PNGs, and file naming.

use std::path::{Path, PathBuf};

/// Regions smaller than this (logical pixels, either side) are treated as
/// a click, not a drag.
pub const MIN_REGION: f32 = 6.0;

/// Tallest full-page capture, in CSS pixels. Chromium can't encode
/// arbitrarily tall surfaces; longer pages are cut off here.
pub const MAX_FULL_PAGE_HEIGHT: f64 = 16_384.0;

/// A rectangle in a page's logical pixels (origin top-left of the page).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Region {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Region {
    /// The rectangle between two drag points, clamped to a page of
    /// `size`. `None` when the drag was too small to mean a region.
    pub fn from_drag(start: (f32, f32), end: (f32, f32), size: (f32, f32)) -> Option<Region> {
        let clamp = |v: f32, max: f32| v.clamp(0.0, max.max(0.0));
        let (x0, x1) = (clamp(start.0.min(end.0), size.0), clamp(start.0.max(end.0), size.0));
        let (y0, y1) = (clamp(start.1.min(end.1), size.1), clamp(start.1.max(end.1), size.1));
        let region = Region {
            x: x0,
            y: y0,
            width: x1 - x0,
            height: y1 - y0,
        };
        (region.width >= MIN_REGION && region.height >= MIN_REGION).then_some(region)
    }

    /// The region in device pixels of a frame `image` pixels large, drawn
    /// at `scale`: `(x, y, width, height)`, inside the frame.
    pub fn to_device(self, scale: f32, image: (u32, u32)) -> Option<(u32, u32, u32, u32)> {
        let to = |v: f32, max: u32| ((v * scale).round().max(0.0) as u32).min(max);
        let (x, y) = (to(self.x, image.0), to(self.y, image.1));
        let (right, bottom) = (to(self.x + self.width, image.0), to(self.y + self.height, image.1));
        (right > x && bottom > y).then_some((x, y, right - x, bottom - y))
    }
}

/// Copy a `(x, y, width, height)` rectangle out of a tightly packed
/// 4-bytes-per-pixel image `width` pixels wide.
pub fn crop(pixels: &[u8], width: u32, rect: (u32, u32, u32, u32)) -> Vec<u8> {
    let (x, y, w, h) = rect;
    let stride = width as usize * 4;
    let mut out = Vec::with_capacity(w as usize * h as usize * 4);
    for row in y..y + h {
        let start = row as usize * stride + x as usize * 4;
        let end = start + w as usize * 4;
        if let Some(line) = pixels.get(start..end) {
            out.extend_from_slice(line);
        }
    }
    out
}

/// Chromium's BGRA frame to opaque RGBA, in place.
pub fn bgra_to_rgba(pixels: &mut [u8]) {
    for px in pixels.as_chunks_mut::<4>().0 {
        px.swap(0, 2);
        px[3] = 255;
    }
}

/// Standard base64 (with or without padding; whitespace ignored), as in
/// DevTools' `Page.captureScreenshot` result. `None` on invalid input.
pub fn decode_base64(text: &str) -> Option<Vec<u8>> {
    fn value(c: u8) -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    }
    let digits: Vec<u8> = text.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    let digits = digits.strip_suffix(b"==").or_else(|| digits.strip_suffix(b"=")).unwrap_or(&digits);
    if digits.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(digits.len() * 3 / 4);
    for chunk in digits.chunks(4) {
        let mut acc = 0u32;
        for (i, c) in chunk.iter().enumerate() {
            acc |= value(*c)? << (18 - 6 * i);
        }
        let bytes = acc.to_be_bytes();
        out.extend_from_slice(&bytes[1..chunk.len()]);
    }
    Some(out)
}

/// Measures the page for a full-page capture (CSS pixels): content height,
/// viewport height and the current scroll position.
pub const FULL_PAGE_MEASURE_SCRIPT: &str = "({ height: Math.max(document.documentElement.scrollHeight, \
    document.body ? document.body.scrollHeight : 0), viewport: window.innerHeight, scroll: window.scrollY })";

/// Hides fixed and sticky elements (headers, cookie bars) so they show once
/// in a stitched full-page capture instead of on every tile.
pub const HIDE_FIXED_SCRIPT: &str = r#"(() => {
  const hidden = [];
  for (const el of document.querySelectorAll('body *')) {
    const p = getComputedStyle(el).position;
    if (p === 'fixed' || p === 'sticky') {
      hidden.push([el, el.style.getPropertyValue('visibility'), el.style.getPropertyPriority('visibility')]);
      el.style.setProperty('visibility', 'hidden', 'important');
    }
  }
  window.__moriCaptureHidden = hidden;
  return hidden.length;
})()"#;

/// Undoes [`HIDE_FIXED_SCRIPT`].
pub const RESTORE_FIXED_SCRIPT: &str = r#"(() => {
  for (const [el, value, priority] of window.__moriCaptureHidden || []) {
    if (value) el.style.setProperty('visibility', value, priority); else el.style.removeProperty('visibility');
  }
  delete window.__moriCaptureHidden;
  return true;
})()"#;

/// Scrolls to `y` at once (ignoring smooth scrolling) and evaluates to the
/// position the page actually took.
pub fn scroll_to_script(y: f64) -> String {
    format!("(window.scrollTo({{ top: {y}, left: window.scrollX, behavior: 'instant' }}), window.scrollY)")
}

/// The scroll positions whose viewports, stacked, cover a page `height`
/// tall seen through a `viewport` tall window: one viewport apart, the
/// last one flush with the bottom. Pages taller than
/// [`MAX_FULL_PAGE_HEIGHT`] are covered up to it; `true` says so.
pub fn scroll_stops(height: f64, viewport: f64) -> (Vec<f64>, bool) {
    let cut_off = height > MAX_FULL_PAGE_HEIGHT;
    let covered = height.min(MAX_FULL_PAGE_HEIGHT);
    if viewport <= 0.0 || covered <= viewport {
        return (vec![0.0], cut_off);
    }
    let last = covered - viewport;
    let mut stops = Vec::new();
    let mut y = 0.0;
    while y < last {
        stops.push(y);
        y += viewport;
    }
    stops.push(last);
    (stops, cut_off)
}

/// "Mori Capture 2026-10-16 at 21.07.33.png" for `secs`: UNIX time shifted
/// into the local time zone (seconds east of UTC added).
pub fn file_name(secs: i64) -> String {
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (y, m, d) = civil_from_days(days);
    format!(
        "Mori Capture {y:04}-{m:02}-{d:02} at {:02}.{:02}.{:02}.png",
        rem / 3600,
        (rem / 60) % 60,
        rem % 60
    )
}

/// `dir/name`, or `dir/<stem> (2).<ext>` and so on if that exists.
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = name.rsplit_once('.').unwrap_or((name, ""));
    (2..)
        .map(|n| {
            let file = if ext.is_empty() { format!("{stem} ({n})") } else { format!("{stem} ({n}).{ext}") };
            dir.join(file)
        })
        .find(|p| !p.exists())
        .expect("some name is free")
}

/// Days since 1970-01-01 to a proleptic Gregorian date (Howard Hinnant).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::test_support::TempDir;

    #[test]
    fn drags_normalize_and_clamp() {
        let r = Region::from_drag((300., 200.), (100., 50.), (1000., 800.)).unwrap();
        assert_eq!(r, Region { x: 100., y: 50., width: 200., height: 150. });
        let r = Region::from_drag((-20., -20.), (50., 2000.), (1000., 800.)).unwrap();
        assert_eq!(r, Region { x: 0., y: 0., width: 50., height: 800. });
        assert!(Region::from_drag((10., 10.), (12., 300.), (1000., 800.)).is_none(), "a click");
        assert!(Region::from_drag((990., 10.), (1200., 300.), (1000., 800.)).is_some());
        assert!(Region::from_drag((1100., 10.), (1200., 300.), (1000., 800.)).is_none(), "outside");
    }

    #[test]
    fn device_rects_scale_and_stay_inside() {
        let r = Region { x: 10., y: 20., width: 100., height: 50. };
        assert_eq!(r.to_device(1.0, (1000, 800)), Some((10, 20, 100, 50)));
        assert_eq!(r.to_device(2.0, (2000, 1600)), Some((20, 40, 200, 100)));
        // A frame not yet resized to the card: cut to what exists.
        assert_eq!(r.to_device(1.0, (60, 800)), Some((10, 20, 50, 50)));
        assert_eq!(r.to_device(1.0, (5, 5)), None);
    }

    #[test]
    fn crop_and_convert() {
        // 3x2 image, pixel value = index.
        let px: Vec<u8> = (0..6u8).flat_map(|i| [i, 10 + i, 20 + i, 0]).collect();
        let c = crop(&px, 3, (1, 0, 2, 2));
        assert_eq!(c, [1, 11, 21, 0, 2, 12, 22, 0, 4, 14, 24, 0, 5, 15, 25, 0]);
        let mut c = c;
        bgra_to_rgba(&mut c);
        assert_eq!(&c[..4], &[21, 11, 1, 255]);
    }

    #[test]
    fn base64() {
        assert_eq!(decode_base64("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(decode_base64("aGVsbG8").unwrap(), b"hello");
        assert_eq!(decode_base64("aGVs\nbG8h").unwrap(), b"hello!");
        assert_eq!(decode_base64("iVBORw0KGgo=").unwrap(), b"\x89PNG\r\n\x1a\n");
        assert_eq!(decode_base64("").unwrap(), b"");
        assert!(decode_base64("a").is_none());
        assert!(decode_base64("a$==").is_none());
    }

    #[test]
    fn scroll_stops_cover_the_page() {
        assert_eq!(scroll_stops(2000., 800.), (vec![0., 800., 1200.], false));
        assert_eq!(scroll_stops(1600., 800.), (vec![0., 800.], false));
        assert_eq!(scroll_stops(500., 800.), (vec![0.], false), "fits in one");
        let (stops, cut) = scroll_stops(100_000., 1000.);
        assert!(cut);
        assert_eq!(*stops.last().unwrap(), MAX_FULL_PAGE_HEIGHT - 1000.);
        assert_eq!(scroll_stops(5000., 0.), (vec![0.], false));
        assert!(scroll_to_script(120.).contains("top: 120"));
    }

    #[test]
    fn names() {
        assert_eq!(file_name(0), "Mori Capture 1970-01-01 at 00.00.00.png");
        assert_eq!(file_name(1_792_184_853), "Mori Capture 2026-10-16 at 21.07.33.png");
        let tmp = TempDir::new();
        assert_eq!(unique_path(tmp.path(), "a.png"), tmp.path().join("a.png"));
        std::fs::write(tmp.path().join("a.png"), b"").unwrap();
        std::fs::write(tmp.path().join("a (2).png"), b"").unwrap();
        assert_eq!(unique_path(tmp.path(), "a.png"), tmp.path().join("a (3).png"));
    }
}
