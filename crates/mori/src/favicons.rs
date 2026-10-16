//! Favicon cache: decoded per host, kept in memory for drawing, and written
//! to `<data>/favicons/<host>.png` so sleeping and restored tabs keep their
//! icons without loading the page. Windows share one cache (an icon one
//! window learns shows in all of them); a private window's cache is its own
//! and reads the saved icons but writes nothing.
//!
//! Hosts asked for but never seen (favorites, pinned and imported tabs that
//! haven't loaded) are fetched in the background from the site itself; see
//! [`Favicons::fetch_wanted`].

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    path::PathBuf,
    rc::Rc,
    sync::{Arc, OnceLock},
    time::Duration,
};

use gpui_kit::*;
use mori_core::favicon_fetch;
use mori_engine::Frame;
use smallvec::SmallVec;
use ureq::ResponseExt as _;

/// A handle to a favicon cache; clones share it.
#[derive(Clone)]
pub struct Favicons(Rc<RefCell<Cache>>);

struct Cache {
    dir: PathBuf,
    /// Icons of new pages are written to `dir`.
    persist: bool,
    images: HashMap<String, Arc<RenderImage>>,
    missing: HashSet<String>,
    /// Hosts drawn without an icon, waiting for a network fetch.
    wanted: Vec<String>,
    /// Hosts already fetched (or being fetched) this session.
    fetched: HashSet<String>,
}

/// Fetched icons are scaled to fit this many pixels on a side.
const FETCHED_SIZE: u32 = 64;
/// Home pages are read up to this many bytes when looking for icon links.
const MAX_PAGE_BYTES: u64 = 512 * 1024;
const MAX_ICON_BYTES: u64 = 1024 * 1024;

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
            wanted: Vec::new(),
            fetched: HashSet::new(),
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
                if cache.persist && favicon_fetch::fetchable_host(host) && !cache.fetched.contains(host) {
                    cache.wanted.push(host.to_string());
                }
                None
            }
        }
    }

    /// The icon file saved for `host`, if there is one.
    pub fn path(&self, host: &str) -> Option<PathBuf> {
        let path = self.0.borrow().dir.join(file_name(host));
        path.exists().then_some(path)
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

impl Favicons {
    /// Fetch icons for the hosts that were drawn without one, off the UI
    /// thread, then redraw every window. Each host is tried once per
    /// session; a page that later loads in a tab still reports its own icon.
    pub fn fetch_wanted(&self, cx: &mut App) {
        let hosts: Vec<String> = {
            let mut cache = self.0.borrow_mut();
            let hosts: Vec<String> = std::mem::take(&mut cache.wanted)
                .into_iter()
                .filter(|h| cache.fetched.insert(h.clone()))
                .collect();
            hosts
        };
        if hosts.is_empty() {
            return;
        }
        let this = self.clone();
        let fetch = cx.background_executor().spawn(async move {
            hosts.into_iter().filter_map(|host| fetch_icon(&host).map(|icon| (host, icon))).collect::<Vec<_>>()
        });
        cx.spawn(async move |cx| {
            let icons = fetch.await;
            if icons.is_empty() {
                return;
            }
            cx.update(|cx| {
                for (host, rgba) in icons {
                    let (width, height) = rgba.dimensions();
                    let mut bgra = rgba.into_raw();
                    for px in bgra.as_chunks_mut::<4>().0 {
                        px.swap(0, 2);
                    }
                    this.insert(&host, Frame { width, height, bgra }, cx);
                }
                cx.refresh_windows();
            });
        })
        .detach();
    }
}

/// The icon of `host`'s home page as RGBA, or `None`. Blocking.
fn fetch_icon(host: &str) -> Option<image::RgbaImage> {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    let agent = AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(8)))
            .user_agent(format!("Mori/{}", env!("CARGO_PKG_VERSION")))
            .build()
            .into()
    });
    let home = format!("https://{host}/");
    let (page_url, html) = match agent.get(&home).call() {
        Ok(mut resp) => {
            let url = resp.get_uri().to_string();
            let html = resp.body_mut().with_config().limit(MAX_PAGE_BYTES).read_to_string().unwrap_or_default();
            (url, html)
        }
        Err(e) => {
            log::debug!("favicon: home page of {host}: {e}");
            (home, String::new())
        }
    };
    favicon_fetch::icon_candidates(&page_url, &html).into_iter().find_map(|url| {
        let bytes = agent.get(&url).call().ok()?.body_mut().with_config().limit(MAX_ICON_BYTES).read_to_vec().ok()?;
        let img = image::load_from_memory(&bytes).ok()?;
        let img = if img.width() > FETCHED_SIZE || img.height() > FETCHED_SIZE {
            img.resize(FETCHED_SIZE, FETCHED_SIZE, image::imageops::FilterType::Lanczos3)
        } else {
            img
        };
        log::debug!("favicon: {host} from {url}");
        Some(img.to_rgba8())
    })
}

fn to_render_image(buffer: image::RgbaImage) -> Arc<RenderImage> {
    let frames: SmallVec<[image::Frame; 1]> = SmallVec::from_elem(image::Frame::new(buffer), 1);
    Arc::new(RenderImage::new(frames))
}
