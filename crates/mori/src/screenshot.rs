//! Capturing pages as PNG: the visible area (the last painted frame, no
//! extra render), a dragged region of it, or the whole page through
//! DevTools' `Page.captureScreenshot`. A capture is copied to the clipboard
//! or saved to the downloads folder, with a toast either way.

use gpui_kit::*;
use mori_core::{capture, workspace::TabId};
use mori_engine::WebView;

use crate::browser::{Browser, ToastAction, ToastStyle};

/// Where a capture goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Destination {
    #[default]
    Clipboard,
    Downloads,
}

/// "Capture Visible Tab": copy the page on screen and save it.
pub fn capture_visible_tab(b: &mut Browser, cx: &mut Context<Browser>) {
    let Some(tab) = b.active_tab() else { return };
    b.capture_frame(tab, None, &[Destination::Clipboard, Destination::Downloads], cx);
}

/// Encode tightly packed RGBA pixels as PNG.
fn encode_png(width: u32, height: u32, rgba: Vec<u8>) -> Option<Vec<u8>> {
    let img = image::RgbaImage::from_raw(width, height, rgba)?;
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
    Some(png)
}

impl Browser {
    /// Capture `tab`'s last painted frame, or `region` of it (logical
    /// pixels from the page's top-left), and send it to `destinations`.
    pub fn capture_frame(
        &mut self,
        tab: TabId,
        region: Option<capture::Region>,
        destinations: &[Destination],
        cx: &mut Context<Self>,
    ) {
        let Some(live) = self.live.get(&tab) else {
            self.toast("Nothing to capture on this page", ToastStyle::Info, None, cx);
            return;
        };
        let surface = live.surface.read(cx);
        let Some((width, height, bgra)) = surface.snapshot_bgra() else {
            self.toast("Nothing to capture yet", ToastStyle::Info, None, cx);
            return;
        };
        // The frame is in device pixels; the page's logical size maps onto it.
        let logical_width = f32::from(surface.bounds().size.width).max(1.);
        let scale = width as f32 / logical_width;
        let rect = match region {
            Some(r) => match r.to_device(scale, (width, height)) {
                Some(rect) => rect,
                None => return,
            },
            None => (0, 0, width, height),
        };
        let destinations = destinations.to_vec();
        let encoded = cx.background_spawn(async move {
            let mut pixels = capture::crop(&bgra, width, rect);
            capture::bgra_to_rgba(&mut pixels);
            encode_png(rect.2, rect.3, pixels)
        });
        cx.spawn(async move |this, cx| {
            let png = encoded.await;
            this.update(cx, |this, cx| match png {
                Some(png) => this.deliver_capture(png, &destinations, false, cx),
                None => this.toast("Couldn't encode the capture", ToastStyle::Error, None, cx),
            })
            .ok();
        })
        .detach();
    }

    /// Capture all of `tab`'s page, beyond the visible area. Offscreen pages
    /// only ever render their viewport, so the page is scrolled through one
    /// viewport at a time (fixed headers hidden after the first) and the
    /// shots are stitched together; the scroll position is put back after.
    pub fn capture_full_page(&mut self, tab: TabId, destination: Destination, cx: &mut Context<Self>) {
        let Some(view) = self.live.get(&tab).map(|l| l.view.clone()) else {
            self.toast("Nothing to capture on this page", ToastStyle::Info, None, cx);
            return;
        };
        self.toast("Capturing the whole page…", ToastStyle::Info, None, cx);
        cx.spawn(async move |this, cx| {
            let result = full_page_png(&view, cx).await;
            this.update(cx, |this, cx| match result {
                Ok((png, cut_off)) => this.deliver_capture(png, &[destination], cut_off, cx),
                Err(e) => {
                    log::warn!("full-page capture: {e}");
                    this.toast("Couldn't capture this page", ToastStyle::Error, None, cx)
                }
            })
            .ok();
        })
        .detach();
    }

    /// Put a captured PNG on the clipboard and/or save it to the downloads
    /// folder, and say so. `cut_off`: a very long page was shortened.
    fn deliver_capture(&mut self, png: Vec<u8>, destinations: &[Destination], cut_off: bool, cx: &mut Context<Self>) {
        let copy = destinations.contains(&Destination::Clipboard);
        let save = destinations.contains(&Destination::Downloads);
        let saved = save.then(|| {
            let dir = self.downloads_dir.clone();
            let local = mori_core::paths::now() as i64 + crate::ui::panels::local_utc_offset();
            let path = capture::unique_path(&dir, &capture::file_name(local));
            std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&path, &png)).map(|_| path)
        });
        let note = if cut_off { " (cut at its first 16,384 px)" } else { "" };
        if !copy {
            match saved {
                Some(Ok(path)) => self.toast(
                    format!("Capture saved to Downloads{note}"),
                    ToastStyle::Success,
                    Some(("Show".into(), ToastAction::Reveal(path))),
                    cx,
                ),
                Some(Err(e)) => self.toast(format!("Couldn't save the capture: {e}"), ToastStyle::Error, None, cx),
                None => {}
            }
            return;
        }
        let image = png.clone();
        let copied = cx.background_spawn(async move { copy_png_to_system_clipboard(&image) });
        cx.spawn(async move |this, cx| {
            let system = copied.await;
            this.update(cx, |this, cx| {
                if !system {
                    // Pasting inside Mori still works.
                    cx.write_to_clipboard(ClipboardItem::new_image(&Image::from_bytes(ImageFormat::Png, png)));
                }
                let copied = if system { "Capture copied" } else { "Capture copied for Mori only (install wl-clipboard)" };
                match saved {
                    Some(Ok(path)) => this.toast(
                        format!("{copied} and saved to Downloads{note}"),
                        ToastStyle::Success,
                        Some(("Show".into(), ToastAction::Reveal(path))),
                        cx,
                    ),
                    Some(Err(e)) => this.toast(format!("{copied}; saving failed: {e}"), ToastStyle::Warning, None, cx),
                    None => this.toast(
                        format!("{copied}{note}"),
                        if system { ToastStyle::Success } else { ToastStyle::Warning },
                        None,
                        cx,
                    ),
                }
            })
            .ok();
        })
        .detach();
    }
}

/// Offer a PNG on the desktop clipboard. GPUI's Linux clipboard only
/// offers text to other apps, so images go through the desktop's own tool
/// (`wl-copy` on Wayland, `xclip` on X11); macOS's clipboard takes images
/// directly. Returns whether another app can now paste it.
fn copy_png_to_system_clipboard(png: &[u8]) -> bool {
    if !cfg!(target_os = "linux") {
        return false;
    }
    use std::{
        io::Write as _,
        process::{Command, Stdio},
    };
    let tool: (&str, &[&str]) = if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        ("wl-copy", &["--type", "image/png"])
    } else if std::env::var_os("DISPLAY").is_some() {
        ("xclip", &["-selection", "clipboard", "-t", "image/png", "-i"])
    } else {
        return false;
    };
    let Ok(mut child) = Command::new(tool.0)
        .args(tool.1)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let written = child.stdin.take().is_some_and(|mut stdin| stdin.write_all(png).is_ok());
    if tool.0 == "wl-copy" {
        // wl-copy returns once it holds the data; a child it forks serves it.
        written && child.wait().is_ok_and(|status| status.success())
    } else {
        // xclip keeps running to serve the selection; reap it when it ends.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        written
    }
}

/// Evaluate `script` in `view`'s page.
async fn evaluate(view: &WebView, script: &str) -> Result<serde_json::Value, String> {
    let (tx, rx) = futures::channel::oneshot::channel();
    view.evaluate(script, move |result| {
        let _ = tx.send(result);
    });
    rx.await.unwrap_or_else(|_| Err("page went away".into()))
}

/// The viewport of `view` as a PNG, once its current scroll is painted.
async fn screenshot(view: &WebView) -> Result<Vec<u8>, String> {
    let (tx, rx) = futures::channel::oneshot::channel();
    view.devtools_call("Page.captureScreenshot", serde_json::json!({ "format": "png" }), move |result| {
        let _ = tx.send(result);
    });
    let shot = rx.await.unwrap_or_else(|_| Err("page went away".into()))?;
    shot.get("data")
        .and_then(|d| d.as_str())
        .and_then(capture::decode_base64)
        .ok_or_else(|| "no image in screenshot".to_string())
}

/// Shoot each viewport down the page: `(scroll position, png)` per shot.
async fn shoot_tiles(view: &WebView, stops: &[f64], tiles: &mut Vec<(f64, Vec<u8>)>) -> Result<(), String> {
    for (i, stop) in stops.iter().enumerate() {
        if i == 1 {
            evaluate(view, capture::HIDE_FIXED_SCRIPT).await?;
        }
        let at = evaluate(view, &capture::scroll_to_script(*stop)).await?.as_f64().unwrap_or(*stop);
        tiles.push((at, screenshot(view).await?));
    }
    Ok(())
}

/// Scroll through the page shooting each viewport, then stitch the shots:
/// `(png, cut_off)`.
async fn full_page_png(view: &WebView, cx: &mut AsyncApp) -> Result<(Vec<u8>, bool), String> {
    let m = evaluate(view, capture::FULL_PAGE_MEASURE_SCRIPT).await?;
    let number = |key: &str| m.get(key).and_then(|v| v.as_f64()).unwrap_or(0.0);
    let (height, viewport, scroll) = (number("height"), number("viewport"), number("scroll"));
    let (stops, cut_off) = capture::scroll_stops(height, viewport);
    let mut tiles = Vec::new();
    let shot = shoot_tiles(view, &stops, &mut tiles).await;
    // Put the page back as it was, whatever happened.
    if stops.len() > 1 {
        let _ = evaluate(view, capture::RESTORE_FIXED_SCRIPT).await;
    }
    let _ = evaluate(view, &capture::scroll_to_script(scroll)).await;
    shot?;
    let covered = height.min(capture::MAX_FULL_PAGE_HEIGHT);
    let png = cx
        .background_spawn(async move { stitch(&tiles, viewport, covered) })
        .await
        .ok_or_else(|| "couldn't stitch the page".to_string())?;
    Ok((png, cut_off))
}

/// Stack viewport shots `(scroll position, png)` into one image of a page
/// `covered` CSS pixels tall.
fn stitch(tiles: &[(f64, Vec<u8>)], viewport: f64, covered: f64) -> Option<Vec<u8>> {
    let decode = |png: &[u8]| {
        image::load_from_memory_with_format(png, image::ImageFormat::Png)
            .ok()
            .map(|i| i.to_rgba8())
    };
    let first = decode(&tiles.first()?.1)?;
    if tiles.len() == 1 {
        return encode_png(first.width(), first.height(), first.into_raw());
    }
    // Device pixels per CSS pixel.
    let density = first.height() as f64 / viewport.max(1.0);
    let mut page = image::RgbaImage::new(first.width(), (covered * density).round() as u32);
    image::imageops::replace(&mut page, &first, 0, (tiles[0].0 * density).round() as i64);
    for (at, png) in &tiles[1..] {
        let tile = decode(png)?;
        image::imageops::replace(&mut page, &tile, 0, (at * density).round() as i64);
    }
    encode_png(page.width(), page.height(), page.into_raw())
}
