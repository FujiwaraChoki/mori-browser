//! Favicon cache: decoded per host, kept in memory for drawing, and written
//! to `<data>/favicons/<host>.png` so sleeping and restored tabs keep their
//! icons without loading the page. Windows share one cache (an icon one
//! window learns shows in all of them); a private window's cache is its own
//! and reads the saved icons but writes nothing.

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    path::PathBuf,
    rc::Rc,
    sync::Arc,
};

use gpui_kit::*;
use mori_engine::Frame;
use smallvec::SmallVec;

/// A handle to a favicon cache; clones share it.
#[derive(Clone)]
pub struct Favicons(Rc<RefCell<Cache>>);

struct Cache {
    dir: PathBuf,
    /// Icons of new pages are written to `dir`.
    persist: bool,
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
        Self::with(dir, true)
    }

    /// A cache that keeps the icons it sees in memory only.
    pub fn in_memory(dir: PathBuf) -> Self {
        Self::with(dir, false)
    }

    fn with(dir: PathBuf, persist: bool) -> Self {
        Favicons(Rc::new(RefCell::new(Cache {
            dir,
            persist,
            images: HashMap::new(),
            missing: HashSet::new(),
        })))
    }

    /// The icon for `host`, loading it from disk on first use.
    pub fn get(&self, host: &str) -> Option<Arc<RenderImage>> {
        let mut cache = self.0.borrow_mut();
        if let Some(img) = cache.images.get(host) {
            return Some(img.clone());
        }
        if cache.missing.contains(host) {
            return None;
        }
        let path = cache.dir.join(file_name(host));
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
                cache.images.insert(host.to_string(), img.clone());
                Some(img)
            }
            None => {
                cache.missing.insert(host.to_string());
                None
            }
        }
    }

    pub fn insert(&self, host: &str, frame: Frame, cx: &mut App) {
        let Frame { width, height, bgra } = frame;
        let Some(buffer) = image::RgbaImage::from_raw(width, height, bgra) else {
            return;
        };
        let mut cache = self.0.borrow_mut();
        if cache.persist {
            // Persist as RGBA PNG off the main thread.
            let mut rgba = buffer.clone();
            for px in rgba.pixels_mut() {
                px.0.swap(0, 2);
            }
            let path = cache.dir.join(file_name(host));
            cx.background_executor()
                .spawn(async move {
                    if let Err(e) = rgba.save(&path) {
                        log::debug!("saving favicon {}: {e}", path.display());
                    }
                })
                .detach();
        }
        cache.missing.remove(host);
        let old = cache.images.insert(host.to_string(), to_render_image(buffer));
        drop(cache);
        if let Some(old) = old {
            cx.drop_image(old, None);
        }
    }
}

fn to_render_image(buffer: image::RgbaImage) -> Arc<RenderImage> {
    let frames: SmallVec<[image::Frame; 1]> = SmallVec::from_elem(image::Frame::new(buffer), 1);
    Arc::new(RenderImage::new(frames))
}
