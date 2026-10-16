//! Favicon cache: decoded per host, kept in memory for drawing, and written
//! to `<data>/favicons/<host>.png` so sleeping and restored tabs keep their
//! icons without loading the page.

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
};

use gpui_kit::*;
use mori_engine::Frame;
use smallvec::SmallVec;

pub struct Favicons {
    dir: PathBuf,
    images: HashMap<String, Arc<RenderImage>>,
    missing: HashSet<String>,
}

fn file_name(host: &str) -> String {
    let safe: String = host
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' })
        .collect();
    format!("{safe}.png")
}

impl Favicons {
    pub fn new(dir: PathBuf) -> Self {
        std::fs::create_dir_all(&dir).ok();
        Favicons {
            dir,
            images: HashMap::new(),
            missing: HashSet::new(),
        }
    }

    /// The icon for `host`, loading it from disk on first use.
    pub fn get(&mut self, host: &str) -> Option<Arc<RenderImage>> {
        if let Some(img) = self.images.get(host) {
            return Some(img.clone());
        }
        if self.missing.contains(host) {
            return None;
        }
        let path = self.dir.join(file_name(host));
        let loaded = image::open(&path).ok().map(|img| {
            let mut rgba = img.to_rgba8();
            // RenderImage expects BGRA.
            for px in rgba.pixels_mut() {
                px.0.swap(0, 2);
            }
            to_render_image(rgba)
        });
        match loaded {
            Some(img) => {
                self.images.insert(host.to_string(), img.clone());
                Some(img)
            }
            None => {
                self.missing.insert(host.to_string());
                None
            }
        }
    }

    pub fn insert(&mut self, host: &str, frame: Frame, cx: &mut App) {
        let Frame { width, height, bgra } = frame;
        let Some(buffer) = image::RgbaImage::from_raw(width, height, bgra) else {
            return;
        };
        // Persist as RGBA PNG off the main thread.
        let mut rgba = buffer.clone();
        for px in rgba.pixels_mut() {
            px.0.swap(0, 2);
        }
        let path = self.dir.join(file_name(host));
        cx.background_executor()
            .spawn(async move {
                if let Err(e) = rgba.save(&path) {
                    log::debug!("saving favicon {}: {e}", path.display());
                }
            })
            .detach();
        self.missing.remove(host);
        if let Some(old) = self.images.insert(host.to_string(), to_render_image(buffer)) {
            cx.drop_image(old, None);
        }
    }
}

fn to_render_image(buffer: image::RgbaImage) -> Arc<RenderImage> {
    let frames: SmallVec<[image::Frame; 1]> = SmallVec::from_elem(image::Frame::new(buffer), 1);
    Arc::new(RenderImage::new(frames))
}
