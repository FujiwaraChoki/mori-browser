//! Mori's built-in ad and tracker blocker.
//!
//! CEF's windowless browsers cannot run Chrome extensions, so Mori blocks
//! ads and trackers itself with Brave's `adblock` engine and the EasyList
//! and EasyPrivacy filter lists:
//!
//! - [`Blocker`] is a compiled filter set: it decides whether a network
//!   request is blocked and which page elements cosmetic filters hide.
//! - [`ContentBlocker`] is the thread-safe switchboard Chromium's network
//!   thread asks for every request (on/off, per-site exceptions, the current
//!   [`Blocker`]).
//! - [`ListCache`] keeps the downloaded lists and a compiled engine in the
//!   data directory and says when they are due for a refresh; until the
//!   first download succeeds the small [`BUNDLED_LIST`] is used.
//! - [`SiteExceptions`] (`content_blocker.json`) are the sites the user
//!   turned blocking off for.

use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, Ordering},
    },
};

use serde::{Deserialize, Serialize};

use crate::{paths, util};

/// Per-site exceptions inside the data directory.
pub const EXCEPTIONS_FILE: &str = "content_blocker.json";
/// Directory (inside the data directory) holding downloaded lists.
pub const LISTS_DIR: &str = "content-blocker";
/// How old downloaded lists may get before they are fetched again. EasyList
/// itself says it expires after four days.
pub const UPDATE_INTERVAL_SECS: f64 = 4.0 * 24.0 * 3600.0;
/// After a failed download, how long to wait before trying again.
pub const RETRY_INTERVAL_SECS: f64 = 3600.0;
/// A downloaded list smaller than this is an error page, not a filter list.
const MIN_LIST_BYTES: usize = 10 * 1024;

/// A filter list Mori downloads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FilterList {
    /// File name stem in [`LISTS_DIR`].
    pub id: &'static str,
    pub name: &'static str,
    pub url: &'static str,
}

/// The lists Mori blocks with: ads (EasyList) and trackers (EasyPrivacy).
pub const LISTS: [FilterList; 2] = [
    FilterList {
        id: "easylist",
        name: "EasyList",
        url: "https://easylist.to/easylist/easylist.txt",
    },
    FilterList {
        id: "easyprivacy",
        name: "EasyPrivacy",
        url: "https://easylist.to/easylist/easyprivacy.txt",
    },
];

/// A small list of the most common ad and tracking networks, compiled into
/// Mori so blocking works from the first launch and while offline.
pub const BUNDLED_LIST: &str = include_str!("content_blocker_default.txt");

// ---- The compiled engine ---------------------------------------------------------

/// A compiled filter set. Immutable and shareable between threads.
pub struct Blocker {
    engine: adblock::Engine,
}

/// Cosmetic filters for one page: CSS hiding the elements its host-specific
/// rules name, plus what generic rules need to know.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cosmetics {
    /// A style sheet hiding the matched elements (may be empty).
    pub css: String,
    /// Class/id selectors generic rules must leave alone on this page.
    pub exceptions: HashSet<String>,
    /// The page opted out of generic cosmetic rules (`$generichide`).
    pub generichide: bool,
}

impl Blocker {
    /// Compile filter list texts (Adblock Plus syntax).
    pub fn from_lists<S: AsRef<str>>(lists: &[S]) -> Blocker {
        let mut set = adblock::lists::FilterSet::new(false);
        for list in lists {
            set.add_filter_list(list.as_ref().to_string(), adblock::lists::ParseOptions::default());
        }
        Blocker { engine: adblock::Engine::new_with_filter_set(set) }
    }

    /// The built-in fallback list.
    pub fn bundled() -> Blocker {
        Blocker::from_lists(&[BUNDLED_LIST])
    }

    /// A compact binary form for [`Blocker::deserialize`].
    pub fn serialize(&self) -> Vec<u8> {
        self.engine.serialize()
    }

    /// Load a blocker saved by [`Blocker::serialize`] (by this version of
    /// Mori; other versions' data is rejected).
    pub fn deserialize(data: &[u8]) -> Option<Blocker> {
        let mut engine = adblock::Engine::default();
        engine.deserialize(data).ok()?;
        Some(Blocker { engine })
    }

    /// Whether a request for `url` made by a page at `source_url` is blocked.
    /// `request_type` is the fetch destination in filter-list terms
    /// (`script`, `image`, `subdocument`, `xmlhttprequest`, …).
    pub fn blocks(&self, url: &str, source_url: &str, request_type: &str) -> bool {
        let Ok(request) = adblock::request::Request::new(url, source_url, request_type, "get") else {
            return false;
        };
        self.engine.check_network_request(&request).should_block()
    }

    /// Host-specific cosmetic filters for the page at `url`.
    pub fn cosmetics(&self, url: &str) -> Cosmetics {
        let resources = self.engine.url_cosmetic_resources(url);
        let mut selectors: Vec<String> = resources.hide_selectors.into_iter().collect();
        selectors.sort();
        Cosmetics {
            css: hiding_css(&selectors),
            exceptions: resources.exceptions,
            generichide: resources.generichide,
        }
    }

    /// CSS for generic cosmetic rules that apply to the classes and ids found
    /// on a page (see [`COLLECT_CLASSES_SCRIPT`]).
    pub fn generic_css(&self, classes: &[String], ids: &[String], exceptions: &HashSet<String>) -> String {
        let mut selectors = self.engine.hidden_class_id_selectors(classes, ids, exceptions);
        selectors.sort();
        hiding_css(&selectors)
    }
}

/// One rule per selector, so a selector Blink rejects only drops its own rule.
fn hiding_css(selectors: &[String]) -> String {
    let mut css = String::new();
    for selector in selectors {
        // A selector carrying a block would escape the rule.
        if selector.contains(['{', '}']) {
            continue;
        }
        css.push_str(selector);
        css.push_str("{display:none!important}\n");
    }
    css
}

// ---- Runtime switchboard ------------------------------------------------------------

/// What Chromium's network thread consults for every request. Cheap to
/// query and safe to share between threads.
pub struct ContentBlocker {
    enabled: AtomicBool,
    blocker: RwLock<Option<Arc<Blocker>>>,
    exceptions: RwLock<BTreeSet<String>>,
}

impl Default for ContentBlocker {
    fn default() -> Self {
        ContentBlocker {
            enabled: AtomicBool::new(true),
            blocker: RwLock::new(None),
            exceptions: RwLock::new(BTreeSet::new()),
        }
    }
}

impl ContentBlocker {
    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::Relaxed);
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    pub fn set_blocker(&self, blocker: Arc<Blocker>) {
        if let Ok(mut slot) = self.blocker.write() {
            *slot = Some(blocker);
        }
    }

    /// The current compiled filters, once loaded.
    pub fn blocker(&self) -> Option<Arc<Blocker>> {
        self.blocker.read().ok().and_then(|b| b.clone())
    }

    pub fn set_exceptions(&self, exceptions: &SiteExceptions) {
        if let Ok(mut slot) = self.exceptions.write() {
            *slot = exceptions.sites.clone();
        }
    }

    /// Whether blocking applies to pages at `page_url` at all: it is on, and
    /// the site is not an exception.
    pub fn active_for(&self, page_url: &str) -> bool {
        if !self.is_enabled() {
            return false;
        }
        let Some(site) = site_key(page_url) else { return true };
        !self.exceptions.read().is_ok_and(|e| e.contains(&site))
    }

    /// Whether to cancel a request for `url` made by the page at
    /// `page_url` (the top-level page). Pages themselves are never blocked.
    pub fn should_block(&self, url: &str, page_url: &str, request_type: &str) -> bool {
        if request_type == "document" || !is_web_url(url) || !self.active_for(page_url) {
            return false;
        }
        self.blocker().is_some_and(|b| b.blocks(url, page_url, request_type))
    }

    /// Cosmetic filters for the page at `url`, when blocking applies to it.
    pub fn cosmetics(&self, url: &str) -> Option<Cosmetics> {
        if !is_web_url(url) || !self.active_for(url) {
            return None;
        }
        self.blocker().map(|b| b.cosmetics(url))
    }
}

fn is_web_url(url: &str) -> bool {
    url.starts_with("https://") || url.starts_with("http://") || url.starts_with("wss://") || url.starts_with("ws://")
}

/// The key a site's exception is stored under: its host without `www.`.
pub fn site_key(url: &str) -> Option<String> {
    if !is_web_url(url) {
        return None;
    }
    util::host_of(url).map(|h| util::strip_www(&h).to_string())
}

// ---- Per-site exceptions ----------------------------------------------------------

/// Sites the user turned the blocker off for.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SiteExceptions {
    pub sites: BTreeSet<String>,
}

impl SiteExceptions {
    pub fn load(dir: &Path) -> SiteExceptions {
        paths::read_json(&dir.join(EXCEPTIONS_FILE)).unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) -> anyhow::Result<()> {
        paths::write_json_atomic(&dir.join(EXCEPTIONS_FILE), self)
    }

    /// Whether blocking is off for the site of `url`.
    pub fn is_exempt(&self, url: &str) -> bool {
        site_key(url).is_some_and(|s| self.sites.contains(&s))
    }

    /// Turn blocking off (`exempt`) or back on for the site of `url`.
    /// Returns whether anything changed.
    pub fn set_exempt(&mut self, url: &str, exempt: bool) -> bool {
        let Some(site) = site_key(url) else { return false };
        if exempt { self.sites.insert(site) } else { self.sites.remove(&site) }
    }

    /// Remove a stored site (as listed in settings).
    pub fn remove_site(&mut self, site: &str) -> bool {
        self.sites.remove(site)
    }
}

// ---- Downloaded lists -------------------------------------------------------------

/// When each list was last downloaded, and the last failed attempt.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ListsMeta {
    /// List id → UNIX seconds of the last successful download.
    pub fetched: BTreeMap<String, f64>,
    pub last_failure: Option<f64>,
    /// Number of network rules in the compiled engine (for settings).
    pub rule_count: usize,
}

/// The downloaded lists and compiled engine in `<data>/content-blocker/`.
pub struct ListCache {
    dir: PathBuf,
}

const META_FILE: &str = "lists.json";
const ENGINE_FILE: &str = "engine.dat";

impl ListCache {
    pub fn new(data_dir: &Path) -> ListCache {
        ListCache { dir: data_dir.join(LISTS_DIR) }
    }

    pub fn meta(&self) -> ListsMeta {
        paths::read_json(&self.dir.join(META_FILE)).unwrap_or_default()
    }

    pub fn save_meta(&self, meta: &ListsMeta) -> anyhow::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        paths::write_json_atomic(&self.dir.join(META_FILE), meta)
    }

    fn list_path(&self, list: &FilterList) -> PathBuf {
        self.dir.join(format!("{}.txt", list.id))
    }

    /// The downloaded text of every list, when all are present.
    pub fn read_lists(&self) -> Option<Vec<String>> {
        LISTS
            .iter()
            .map(|l| std::fs::read_to_string(self.list_path(l)).ok())
            .collect()
    }

    /// Store a freshly downloaded list. Text that doesn't look like a filter
    /// list (an error or captive-portal page) is rejected.
    pub fn store_list(&self, list: &FilterList, text: &str) -> anyhow::Result<()> {
        anyhow::ensure!(looks_like_filter_list(text), "{} download is not a filter list", list.name);
        std::fs::create_dir_all(&self.dir)?;
        paths::write_bytes_atomic(&self.list_path(list), text.as_bytes())
    }

    /// The compiled engine saved by [`ListCache::save_engine`], if still
    /// loadable.
    pub fn load_engine(&self) -> Option<Blocker> {
        let data = std::fs::read(self.dir.join(ENGINE_FILE)).ok()?;
        Blocker::deserialize(&data)
    }

    pub fn save_engine(&self, blocker: &Blocker) -> anyhow::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        paths::write_bytes_atomic(&self.dir.join(ENGINE_FILE), &blocker.serialize())
    }
}

/// Whether `text` is plausibly an Adblock Plus filter list.
pub fn looks_like_filter_list(text: &str) -> bool {
    text.len() >= MIN_LIST_BYTES && text.trim_start().starts_with("[Adblock")
}

/// The lists due for download at `now`: missing or older than
/// [`UPDATE_INTERVAL_SECS`]. Nothing is due within [`RETRY_INTERVAL_SECS`]
/// of a failed attempt.
pub fn lists_due(meta: &ListsMeta, now: f64) -> Vec<FilterList> {
    if meta.last_failure.is_some_and(|t| now - t < RETRY_INTERVAL_SECS && now >= t) {
        return Vec::new();
    }
    LISTS
        .iter()
        .filter(|l| meta.fetched.get(l.id).is_none_or(|t| now - t >= UPDATE_INTERVAL_SECS || now < *t))
        .copied()
        .collect()
}

/// Number of network-blocking rules in list texts (comments, cosmetic rules
/// and exceptions not counted), for the settings page.
pub fn count_network_rules<S: AsRef<str>>(lists: &[S]) -> usize {
    lists
        .iter()
        .flat_map(|l| l.as_ref().lines())
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('!') && !l.starts_with('[') && !l.starts_with("@@"))
        .filter(|l| !l.contains("##") && !l.contains("#@#") && !l.contains("#?#") && !l.contains("#$#"))
        .count()
}

// ---- Page scripts -------------------------------------------------------------------

/// The id of the `<style>` element cosmetic filters live in.
const STYLE_ID: &str = "__mori-cosmetic";

/// A script putting `css` into the page's cosmetic style sheet (`slot`
/// keeps host-specific and generic rules apart), replacing what it held.
/// Works before `<html>` exists.
pub fn inject_css_script(slot: &str, css: &str) -> String {
    let css = serde_json::to_string(css).unwrap_or_else(|_| "\"\"".into());
    let id = format!("{STYLE_ID}-{}", slot.replace(|c: char| !c.is_ascii_alphanumeric(), ""));
    format!(
        r#"(() => {{
  const css = {css};
  const put = () => {{
    let style = document.getElementById("{id}");
    if (!style) {{
      style = document.createElement('style');
      style.id = "{id}";
      document.documentElement.appendChild(style);
    }}
    if (style.textContent !== css) style.textContent = css;
  }};
  if (document.documentElement) put();
  else new MutationObserver((_, o) => {{ if (document.documentElement) {{ o.disconnect(); put(); }} }})
    .observe(document, {{ childList: true }});
}})()"#
    )
}

/// Evaluates to `{classes, ids}`: the distinct class names and ids on the
/// page (capped), for [`Blocker::generic_css`].
pub const COLLECT_CLASSES_SCRIPT: &str = r#"(() => {
  const classes = new Set(), ids = new Set(), max = 6000;
  for (const el of document.querySelectorAll('[id],[class]')) {
    if (el.id && ids.size < max) ids.add(el.id);
    if (el.classList) for (const c of el.classList) { if (classes.size < max) classes.add(c); }
    if (classes.size >= max && ids.size >= max) break;
  }
  return { classes: [...classes], ids: [...ids] };
})()"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::test_support::TempDir;

    fn blocker() -> Blocker {
        Blocker::from_lists(&[
            "||ads.example.net^\n@@||ads.example.net/allowed.js\n||tracker.test^$third-party\nnews.test##.banner-ad\n##.adsbygoogle",
        ])
    }

    #[test]
    fn blocks_network_requests() {
        let b = blocker();
        let page = "https://news.test/story";
        assert!(b.blocks("https://ads.example.net/ad.js", page, "script"));
        assert!(!b.blocks("https://ads.example.net/allowed.js", page, "script"), "exception wins");
        assert!(b.blocks("https://tracker.test/pixel.gif", page, "image"));
        assert!(!b.blocks("https://tracker.test/app.js", "https://tracker.test/", "script"), "first party");
        assert!(!b.blocks("https://cdn.news.test/app.js", page, "script"));
    }

    #[test]
    fn bundled_list_blocks_the_big_networks() {
        let b = Blocker::bundled();
        let page = "https://example.com/";
        assert!(b.blocks("https://securepubads.g.doubleclick.net/tag/js/gpt.js", page, "script"));
        assert!(b.blocks("https://www.google-analytics.com/analytics.js", page, "script"));
        assert!(b.blocks("https://pagead2.googlesyndication.com/pagead/js/adsbygoogle.js", page, "script"));
        assert!(!b.blocks("https://www.google.com/", page, "subdocument"));
        assert!(!b.blocks("https://fonts.gstatic.com/s/inter.woff2", page, "font"));
    }

    #[test]
    fn cosmetic_rules() {
        let b = blocker();
        let c = b.cosmetics("https://news.test/a");
        assert_eq!(c.css, ".banner-ad{display:none!important}\n");
        assert!(b.cosmetics("https://other.test/").css.is_empty());
        let generic = b.generic_css(&["adsbygoogle".into(), "story".into()], &[], &c.exceptions);
        assert_eq!(generic, ".adsbygoogle{display:none!important}\n");
        assert_eq!(hiding_css(&["a{}b".into(), "#x".into()]), "#x{display:none!important}\n");
    }

    #[test]
    fn serializes() {
        let b = blocker();
        let back = Blocker::deserialize(&b.serialize()).expect("roundtrip");
        assert!(back.blocks("https://ads.example.net/ad.js", "https://news.test/", "script"));
        assert!(Blocker::deserialize(b"garbage").is_none());
    }

    #[test]
    fn switchboard_respects_switch_and_exceptions() {
        let cb = ContentBlocker::default();
        let page = "https://www.news.test/story";
        assert!(!cb.should_block("https://ads.example.net/ad.js", page, "script"), "no engine yet");
        cb.set_blocker(Arc::new(blocker()));
        assert!(cb.should_block("https://ads.example.net/ad.js", page, "script"));
        assert!(!cb.should_block("https://ads.example.net/", page, "document"), "pages are never blocked");
        assert!(!cb.should_block("data:text/plain,x", page, "image"));
        let mut ex = SiteExceptions::default();
        assert!(ex.set_exempt("https://news.test/other", true));
        assert!(!ex.set_exempt(page, true), "www is the same site");
        cb.set_exceptions(&ex);
        assert!(!cb.should_block("https://ads.example.net/ad.js", page, "script"));
        assert!(cb.cosmetics(page).is_none());
        assert!(cb.should_block("https://ads.example.net/ad.js", "https://elsewhere.test/", "script"));
        cb.set_enabled(false);
        assert!(!cb.should_block("https://ads.example.net/ad.js", "https://elsewhere.test/", "script"));
        assert!(!cb.active_for("https://elsewhere.test/"));
    }

    #[test]
    fn exceptions_roundtrip() {
        let tmp = TempDir::new();
        let mut ex = SiteExceptions::default();
        ex.set_exempt("https://www.youtube.com/watch", true);
        assert!(ex.is_exempt("https://youtube.com/"));
        assert!(!ex.is_exempt("https://m.youtube.com/"), "per host");
        assert!(!ex.set_exempt("mori://settings", true));
        ex.save(tmp.path()).unwrap();
        assert_eq!(SiteExceptions::load(tmp.path()), ex);
        assert!(ex.remove_site("youtube.com"));
        assert!(!ex.set_exempt("https://youtube.com/", false));
    }

    #[test]
    fn update_schedule() {
        let now = 1_000_000.0;
        let mut meta = ListsMeta::default();
        assert_eq!(lists_due(&meta, now).len(), 2, "never downloaded");
        meta.fetched.insert("easylist".into(), now - 3600.0);
        meta.fetched.insert("easyprivacy".into(), now - UPDATE_INTERVAL_SECS - 1.0);
        assert_eq!(lists_due(&meta, now), vec![LISTS[1]]);
        meta.last_failure = Some(now - 60.0);
        assert!(lists_due(&meta, now).is_empty(), "backing off after a failure");
        meta.last_failure = Some(now - RETRY_INTERVAL_SECS);
        assert_eq!(lists_due(&meta, now).len(), 1);
        meta.fetched.insert("easylist".into(), now + 9999.0);
        assert_eq!(lists_due(&meta, now).len(), 2, "clock went backwards");
    }

    #[test]
    fn list_cache() {
        let tmp = TempDir::new();
        let cache = ListCache::new(tmp.path());
        assert!(cache.read_lists().is_none());
        assert!(cache.store_list(&LISTS[0], "<html>captive portal</html>").is_err());
        let text = format!("[Adblock Plus 2.0]\n! Title: test\n{}", "||ads.example.net^\n".repeat(800));
        cache.store_list(&LISTS[0], &text).unwrap();
        assert!(cache.read_lists().is_none(), "one list still missing");
        cache.store_list(&LISTS[1], &text).unwrap();
        assert_eq!(cache.read_lists().unwrap().len(), 2);
        assert_eq!(count_network_rules(&cache.read_lists().unwrap()), 1600);
        let b = Blocker::from_lists(&cache.read_lists().unwrap());
        cache.save_engine(&b).unwrap();
        assert!(cache.load_engine().unwrap().blocks("https://ads.example.net/x", "https://a.test/", "image"));
        let meta = ListsMeta { rule_count: 3, ..Default::default() };
        cache.save_meta(&meta).unwrap();
        assert_eq!(cache.meta(), meta);
    }

    #[test]
    fn rule_counting() {
        let list = "[Adblock Plus]\n! comment\n||a.test^\n@@||b.test^\nexample.com##.ad\n\n/banner/*\n";
        assert_eq!(count_network_rules(&[list]), 2);
    }

    #[test]
    fn css_injection_script_is_escaped() {
        let js = inject_css_script("host", "a[title=\"x\"]{display:none}</style>");
        assert!(js.contains(r#"const css = "a[title=\"x\"]{display:none}</style>";"#));
        assert!(js.contains("__mori-cosmetic-host"));
    }
}
