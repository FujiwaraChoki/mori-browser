//! "Capture Visible Tab": save the page's last painted frame as a PNG and
//! put it on the clipboard. The frame is already in memory, so no extra
//! render is needed.

use gpui_kit::*;

use crate::browser::{Browser, ToastStyle};

pub fn capture_visible_tab(b: &mut Browser, cx: &mut Context<Browser>) {
    let Some(live) = b.selected_live() else {
        b.toast("Nothing to capture on this page", ToastStyle::Info, None, cx);
        return;
    };
    let Some((width, height, mut bytes)) = live.surface.read(cx).snapshot_bgra() else {
        b.toast("Nothing to capture yet", ToastStyle::Info, None, cx);
        return;
    };
    for px in bytes.as_chunks_mut::<4>().0 {
        px.swap(0, 2); // BGRA -> RGBA
        px[3] = 255;
    }
    let Some(img) = image::RgbaImage::from_raw(width, height, bytes) else {
        return;
    };
    let mut png = Vec::new();
    if img
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .is_err()
    {
        b.toast("Couldn't encode screenshot", ToastStyle::Error, None, cx);
        return;
    }
    cx.write_to_clipboard(ClipboardItem::new_image(&Image::from_bytes(
        ImageFormat::Png,
        png.clone(),
    )));
    let dir = dirs::picture_dir()
        .or_else(dirs::desktop_dir)
        .or_else(dirs::home_dir)
        .unwrap_or_default();
    let stamp = chrono_like_stamp();
    let path = dir.join(format!("Mori Shot {stamp}.png"));
    match std::fs::write(&path, png) {
        Ok(()) => b.toast(
            format!("Screenshot copied & saved to {}", dir.display()),
            ToastStyle::Success,
            None,
            cx,
        ),
        Err(e) => b.toast(format!("Copied; saving failed: {e}"), ToastStyle::Warning, None, cx),
    }
}

/// `YYYY-MM-DD at HH.MM.SS` in UTC (no timezone database needed).
fn chrono_like_stamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02} at {:02}.{:02}.{:02}",
        rem / 3600,
        (rem / 60) % 60,
        rem % 60
    )
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
    #[test]
    fn civil_dates() {
        assert_eq!(super::civil_from_days(0), (1970, 1, 1));
        assert_eq!(super::civil_from_days(20_742), (2026, 10, 16));
    }
}
