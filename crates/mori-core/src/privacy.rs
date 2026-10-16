//! Privacy rules: clearing browsing data, grouping cookies by site, web
//! notifications, and the page bridge script that lets pages reach Mori for
//! the permission-gated features Chromium can't offer offscreen.
//!
//! Chromium auto-denies every permission prompt under the headless ozone
//! platform Mori renders with (`display::Screen::IsHeadless`), and CEF has
//! no notification presenter. So Mori installs [`PAGE_BRIDGE_SCRIPT`] into
//! every top-level document: it routes `Notification`, geolocation,
//! clipboard reads and Picture-in-Picture through Mori, which asks the user
//! with its own prompt and records the answer as a Chromium content setting
//! (so `Notification.permission` and the permissions API agree).

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

use crate::util;

// ---- Clear browsing data ---------------------------------------------------------

/// How far back "Clear browsing data" reaches.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TimeRange {
    LastHour,
    #[default]
    LastDay,
    LastWeek,
    LastFourWeeks,
    AllTime,
}

impl TimeRange {
    pub const ALL: [TimeRange; 5] = [
        TimeRange::LastHour,
        TimeRange::LastDay,
        TimeRange::LastWeek,
        TimeRange::LastFourWeeks,
        TimeRange::AllTime,
    ];

    pub fn label(self) -> &'static str {
        match self {
            TimeRange::LastHour => "Last hour",
            TimeRange::LastDay => "Last 24 hours",
            TimeRange::LastWeek => "Last 7 days",
            TimeRange::LastFourWeeks => "Last 4 weeks",
            TimeRange::AllTime => "All time",
        }
    }

    /// The earliest time (UNIX seconds) data is cleared from, or `None` for
    /// everything.
    pub fn since(self, now: f64) -> Option<f64> {
        let hours = match self {
            TimeRange::LastHour => 1.0,
            TimeRange::LastDay => 24.0,
            TimeRange::LastWeek => 24.0 * 7.0,
            TimeRange::LastFourWeeks => 24.0 * 28.0,
            TimeRange::AllTime => return None,
        };
        Some(now - hours * 3600.0)
    }
}

/// What "Clear browsing data" removes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClearOptions {
    pub history: bool,
    pub cookies: bool,
    pub cache: bool,
    pub downloads: bool,
    pub range: TimeRange,
}

impl Default for ClearOptions {
    fn default() -> Self {
        ClearOptions { history: true, cookies: true, cache: true, downloads: false, range: TimeRange::default() }
    }
}

impl ClearOptions {
    pub fn any(&self) -> bool {
        self.history || self.cookies || self.cache || self.downloads
    }

    /// A one-line summary of what was cleared, for a toast.
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if self.history {
            parts.push("history");
        }
        if self.cookies {
            parts.push("cookies");
        }
        if self.cache {
            parts.push("cache");
        }
        if self.downloads {
            parts.push("downloads list");
        }
        let what = match parts.as_slice() {
            [] => return "Nothing to clear".into(),
            [one] => one.to_string(),
            [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
        };
        let mut s = format!("Cleared {what}");
        if let Some(first) = s.get_mut(..1) {
            first.make_ascii_uppercase();
        }
        s
    }
}

/// The origins whose stored data (local storage, IndexedDB, caches, service
/// workers) is cleared along with cookies: every http(s) origin of `urls`.
pub fn origins_of<'a>(urls: impl IntoIterator<Item = &'a str>) -> BTreeSet<String> {
    urls.into_iter().filter_map(crate::site_permissions::origin_of).collect()
}

// ---- Cookies and site data ------------------------------------------------------

/// The site (registrable domain, e.g. `bbc.co.uk`) a host or cookie domain
/// belongs to. Hosts without a public suffix (IP addresses, `localhost`)
/// are their own site.
pub fn site_of_host(host: &str) -> String {
    let host = host.trim().trim_start_matches('.').trim_end_matches('.').to_ascii_lowercase();
    if host.parse::<std::net::IpAddr>().is_ok() {
        return host;
    }
    match addr::parse_domain_name(&host) {
        Ok(name) => name.root().map(str::to_string).unwrap_or(host),
        Err(_) => host,
    }
}

/// Whether a cookie for `cookie_domain` belongs to `site`.
pub fn cookie_in_site(cookie_domain: &str, site: &str) -> bool {
    let domain = cookie_domain.trim_start_matches('.').to_ascii_lowercase();
    util::host_matches(&domain, site)
}

/// Cookie counts per site, sorted by site, for the cookies viewer.
pub fn cookie_sites<'a>(domains: impl IntoIterator<Item = &'a str>) -> Vec<(String, usize)> {
    let mut sites: BTreeMap<String, usize> = BTreeMap::new();
    for domain in domains {
        *sites.entry(site_of_host(domain)).or_default() += 1;
    }
    sites.into_iter().collect()
}

/// A short human size, e.g. `1.2 MB`.
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = "B";
    for u in UNITS {
        if value < 1000.0 {
            break;
        }
        value /= 1000.0;
        unit = u;
    }
    if value < 10.0 { format!("{value:.1} {unit}") } else { format!("{value:.0} {unit}") }
}

// ---- Web notifications ---------------------------------------------------------

/// Longest notification title and body Mori shows.
const MAX_TITLE_CHARS: usize = 120;
const MAX_BODY_CHARS: usize = 400;

/// Title and body for a desktop notification: trimmed, shortened, and with
/// the body escaped for notification servers that render markup.
pub fn notification_text(title: &str, body: &str) -> (String, String) {
    let clean = |s: &str, max: usize| {
        let one_line: String = s.chars().map(|c| if c.is_control() && c != '\n' { ' ' } else { c }).collect();
        let trimmed = one_line.trim();
        let cut = util::truncate_chars(trimmed, max);
        if cut.len() < trimmed.len() { format!("{}…", cut.trim_end()) } else { cut.to_string() }
    };
    let body = clean(body, MAX_BODY_CHARS).replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    (clean(title, MAX_TITLE_CHARS), body)
}

/// The app name a site's notifications show under: its host without `www.`.
pub fn notification_source(url: &str) -> String {
    util::host_of(url).map(|h| util::strip_www(&h).to_string()).unwrap_or_else(|| "Mori".into())
}

// ---- The page bridge -----------------------------------------------------------

/// Console messages starting with this carry bridge messages (JSON follows).
pub const BRIDGE_MARKER: &str = "\u{1}mori-bridge:";
/// Placeholder in [`PAGE_BRIDGE_SCRIPT`] for the per-page secret that proves
/// a message came from the bridge (not from other script on the page).
pub const BRIDGE_TOKEN_PLACEHOLDER: &str = "__MORI_BRIDGE_TOKEN__";
/// Placeholder for whether Do Not Track / Global Privacy Control is on.
const DNT_PLACEHOLDER: &str = "__MORI_DNT__";

/// A message from the bridge script, minus the token (checked by the
/// engine).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "t", rename_all = "kebab-case")]
pub enum BridgeMessage {
    /// `new Notification()` or `registration.showNotification()`.
    Notify {
        nid: u64,
        #[serde(default)]
        title: String,
        #[serde(default)]
        body: String,
        #[serde(default)]
        tag: String,
        #[serde(default)]
        silent: bool,
    },
    /// `notification.close()`.
    NotifyClose { nid: u64 },
    /// `navigator.clipboard.readText()` after the clipboard permission was
    /// granted; answer with the text.
    ClipRead { rid: u64 },
    /// `video.requestPictureInPicture()` (`open`) or
    /// `document.exitPictureInPicture()`.
    Pip {
        #[serde(default)]
        rid: Option<u64>,
        open: bool,
    },
}

impl BridgeMessage {
    pub fn parse(value: &serde_json::Value) -> Option<BridgeMessage> {
        serde_json::from_value(value.clone()).ok()
    }
}

/// Script that answers bridge request `rid` with `value`.
pub fn bridge_reply_script(rid: u64, value: &serde_json::Value) -> String {
    format!("window.__moriReply && window.__moriReply({rid}, {value})")
}

/// Script that delivers an event (`{t: …}`) to the bridge, e.g. a
/// notification click.
pub fn bridge_event_script(event: &serde_json::Value) -> String {
    format!("window.__moriEvent && window.__moriEvent({event})")
}

/// The bridge script with the Do Not Track choice filled in. The engine
/// fills in the token per page.
pub fn page_bridge_script(do_not_track: bool) -> String {
    PAGE_BRIDGE_SCRIPT
        .replace(DNT_PLACEHOLDER, if do_not_track { "true" } else { "false" })
        .replace("__MARKER__", &serde_json::to_string(BRIDGE_MARKER).unwrap_or_default())
}

/// Installed into every document before its own scripts run. Requests
/// carry the page's token; replies come back through `__moriReply` and
/// events through `__moriEvent`.
const PAGE_BRIDGE_SCRIPT: &str = r#"(() => {
  if (__MORI_DNT__) {
    try {
      Object.defineProperty(Navigator.prototype, 'doNotTrack', { get: () => '1', configurable: true });
      Object.defineProperty(Navigator.prototype, 'globalPrivacyControl', { get: () => true, configurable: true });
    } catch (e) {}
  }
  if (window.top !== window || window.__moriBridgeInstalled) return;
  Object.defineProperty(window, '__moriBridgeInstalled', { value: true });
  const log = console.debug.bind(console);
  const marker = __MARKER__;
  const token = "__MORI_BRIDGE_TOKEN__";
  // The token must not reach page script: the message is copied into an
  // object without a prototype (no inherited `token` setter or `toJSON`
  // sees it) and serialised with the original JSON.stringify.
  const stringify = JSON.stringify;
  const assign = Object.assign;
  const bare = () => Object.create(null);
  const send = (msg) => {
    try {
      const m = assign(bare(), msg);
      m.token = token;
      log(marker + stringify(m));
    } catch (e) {}
  };
  // Random start: a stale answer meant for the previous document can't
  // match a request of this one.
  let nextId = Math.floor(Math.random() * 1e9) + 1;
  const pending = new Map();
  const request = (msg) => new Promise((resolve) => {
    const rid = nextId++;
    pending.set(rid, resolve);
    const m = assign(bare(), msg);
    m.rid = rid;
    send(m);
  });
  const hidden = (name, value) => Object.defineProperty(window, name, { value, enumerable: false });
  const handlers = {};
  hidden('__moriReply', (rid, value) => {
    const resolve = pending.get(rid);
    if (resolve) { pending.delete(rid); resolve(value); }
  });
  hidden('__moriEvent', (ev) => { const h = ev && handlers[ev.t]; if (h) h(ev); });
  const askPermission = (kind) => request({ t: 'perm', kind });
  // Replaced functions still read as built-ins.
  const native = (f, name) => {
    try {
      Object.defineProperty(f, 'toString', { value: () => `function ${name}() { [native code] }` });
    } catch (e) {}
    return f;
  };
  // Like Chrome, notifications, location and clipboard reads are only for
  // secure pages; elsewhere Chromium's own (denying) versions stay.
  const secure = window.isSecureContext;

  // Notifications: shown on the desktop by Mori.
  const NativeNotification = window.Notification;
  if (NativeNotification && secure) {
    const nativePermission = Object.getOwnPropertyDescriptor(NativeNotification, 'permission');
    // Chromium's content setting (which Mori writes before answering) is
    // the truth, so a change in Mori's site settings shows at once; the
    // last answer only stands in if it can't be read.
    let decided = null;
    const permission = () => {
      try { return nativePermission.get.call(NativeNotification); } catch (e) { return decided || 'default'; }
    };
    const live = new Map();
    const fire = (n, type) => {
      const ev = new Event(type, { cancelable: type === 'click' });
      const handler = n['on' + type];
      if (typeof handler === 'function') { try { handler.call(n, ev); } catch (e) { setTimeout(() => { throw e; }); } }
      n.dispatchEvent(ev);
    };
    const toMessage = (nid, title, options) => ({
      t: 'notify', nid, title: String(title),
      body: options && options.body != null ? String(options.body) : '',
      tag: options && options.tag != null ? String(options.tag) : '',
      silent: !!(options && options.silent),
    });
    class Notification extends EventTarget {
      constructor(title, options = {}) {
        super();
        if (arguments.length === 0) throw new TypeError("Failed to construct 'Notification': 1 argument required, but only 0 present.");
        options = options || {};
        Object.defineProperties(this, {
          title: { value: String(title), enumerable: true },
          body: { value: options.body != null ? String(options.body) : '', enumerable: true },
          tag: { value: options.tag != null ? String(options.tag) : '', enumerable: true },
          icon: { value: options.icon ? String(options.icon) : '', enumerable: true },
          data: { value: options.data === undefined ? null : options.data, enumerable: true },
          silent: { value: !!options.silent, enumerable: true },
          requireInteraction: { value: !!options.requireInteraction, enumerable: true },
        });
        this.onclick = this.onshow = this.onclose = this.onerror = null;
        const nid = nextId++;
        Object.defineProperty(this, '__nid', { value: nid });
        if (permission() !== 'granted') { setTimeout(() => fire(this, 'error')); return; }
        live.set(nid, this);
        send(toMessage(nid, title, options));
        setTimeout(() => fire(this, 'show'));
      }
      close() {
        if (!live.delete(this.__nid)) return;
        send({ t: 'notify-close', nid: this.__nid });
        setTimeout(() => fire(this, 'close'));
      }
      static get permission() { return permission(); }
      static get maxActions() { return 0; }
      static requestPermission(callback) {
        return askPermission('notifications').then((result) => {
          decided = result === 'granted' ? 'granted' : 'denied';
          if (typeof callback === 'function') callback(decided);
          return decided;
        });
      }
    }
    handlers['notification-click'] = (ev) => {
      const n = live.get(ev.nid);
      if (n) fire(n, 'click');
    };
    handlers['notification-close'] = (ev) => {
      const n = live.get(ev.nid);
      if (n && live.delete(ev.nid)) fire(n, 'close');
    };
    native(Notification, 'Notification');
    native(Notification.requestPermission, 'requestPermission');
    Object.defineProperty(window, 'Notification', { value: Notification, writable: true, configurable: true });
    if (window.ServiceWorkerRegistration) {
      const proto = ServiceWorkerRegistration.prototype;
      proto.showNotification = function (title, options) {
        if (permission() !== 'granted') {
          return Promise.reject(new TypeError('No notification permission has been granted for this origin.'));
        }
        send(toMessage(nextId++, title, options));
        return Promise.resolve();
      };
      proto.getNotifications = function () { return Promise.resolve([]); };
      native(proto.showNotification, 'showNotification');
      native(proto.getNotifications, 'getNotifications');
    }
  }

  // Geolocation: Mori asks, then lets Chromium's own lookup run.
  const geo = navigator.geolocation;
  if (geo && secure) {
    const get = geo.getCurrentPosition.bind(geo);
    const watch = geo.watchPosition.bind(geo);
    const clear = geo.clearWatch.bind(geo);
    const denied = (error) => {
      if (typeof error !== 'function') return;
      const proto = window.GeolocationPositionError ? GeolocationPositionError.prototype : Object.prototype;
      error(Object.create(proto, {
        code: { value: 1 }, message: { value: 'User denied Geolocation' },
        PERMISSION_DENIED: { value: 1 }, POSITION_UNAVAILABLE: { value: 2 }, TIMEOUT: { value: 3 },
      }));
    };
    const watches = new Map();
    let nextWatch = 1;
    geo.getCurrentPosition = function (success, error, options) {
      askPermission('geolocation').then((r) => (r === 'granted' ? get(success, error, options) : denied(error)));
    };
    geo.watchPosition = function (success, error, options) {
      const id = nextWatch++;
      watches.set(id, null);
      askPermission('geolocation').then((r) => {
        if (!watches.has(id)) return;
        if (r === 'granted') watches.set(id, watch(success, error, options));
        else denied(error);
      });
      return id;
    };
    geo.clearWatch = function (watchId) {
      const id = watches.get(watchId);
      watches.delete(watchId);
      if (id != null) clear(id);
    };
    native(geo.getCurrentPosition, 'getCurrentPosition');
    native(geo.watchPosition, 'watchPosition');
    native(geo.clearWatch, 'clearWatch');
  }

  // Clipboard reads come from the system clipboard, through Mori.
  const clipboard = navigator.clipboard;
  if (clipboard && secure) {
    const hasFocus = Document.prototype.hasFocus;
    const readText = () => askPermission('clipboard').then((r) => {
      if (!hasFocus.call(document)) throw new DOMException('Document is not focused.', 'NotAllowedError');
      if (r !== 'granted') throw new DOMException('Read permission denied.', 'NotAllowedError');
      return request({ t: 'clip-read' }).then((text) => {
        if (typeof text !== 'string') throw new DOMException('Document is not focused.', 'NotAllowedError');
        return text;
      });
    });
    clipboard.readText = function () { return readText(); };
    clipboard.read = function () {
      return readText().then((text) => [new ClipboardItem({ 'text/plain': new Blob([text], { type: 'text/plain' }) })]);
    };
    native(clipboard.readText, 'readText');
    native(clipboard.read, 'read');
  }

  // Picture-in-Picture: Mori's own floating player.
  if (window.HTMLVideoElement) {
    let pipElement = null;
    const leave = () => {
      const el = pipElement;
      pipElement = null;
      if (el) el.dispatchEvent(new Event('leavepictureinpicture', { bubbles: true }));
    };
    handlers['pip-left'] = leave;
    const activation = navigator.userActivation;
    HTMLVideoElement.prototype.requestPictureInPicture = function () {
      const el = this;
      // Chrome's rules: not for videos that opt out or have no metadata
      // yet, and only from a user gesture unless already in
      // Picture-in-Picture.
      if (el.disablePictureInPicture) {
        return Promise.reject(new DOMException('"disablePictureInPicture" attribute is present.', 'InvalidStateError'));
      }
      if (el.readyState === 0) {
        return Promise.reject(new DOMException('Metadata for the video element are not loaded yet.', 'InvalidStateError'));
      }
      if (!pipElement && activation && !activation.isActive) {
        return Promise.reject(new DOMException(
          "Must be handling a user gesture if there isn't already an element in Picture-in-Picture.", 'NotAllowedError'));
      }
      if (pipElement && pipElement !== el) leave();
      el.__moriMediaUserSelected = true;
      pipElement = el;
      return request({ t: 'pip', open: true }).then((ok) => {
        if (!ok) {
          pipElement = null;
          throw new DOMException('Picture-in-Picture is not available.', 'NotAllowedError');
        }
        el.dispatchEvent(new Event('enterpictureinpicture', { bubbles: true }));
        return { width: el.videoWidth, height: el.videoHeight, onresize: null,
                 addEventListener() {}, removeEventListener() {} };
      });
    };
    Object.defineProperty(Document.prototype, 'pictureInPictureElement', { get: () => pipElement, configurable: true });
    Object.defineProperty(Document.prototype, 'pictureInPictureEnabled', { get: () => true, configurable: true });
    Document.prototype.exitPictureInPicture = function () {
      if (!pipElement) return Promise.reject(new DOMException('There is no Picture-in-Picture element.', 'InvalidStateError'));
      send({ t: 'pip', open: false });
      leave();
      return Promise.resolve();
    };
    native(HTMLVideoElement.prototype.requestPictureInPicture, 'requestPictureInPicture');
    native(Document.prototype.exitPictureInPicture, 'exitPictureInPicture');
  }
})();"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_ranges() {
        let now = 1_000_000.0;
        assert_eq!(TimeRange::LastHour.since(now), Some(now - 3600.0));
        assert_eq!(TimeRange::LastWeek.since(now), Some(now - 7.0 * 86400.0));
        assert_eq!(TimeRange::AllTime.since(now), None);
        assert_eq!(TimeRange::ALL.len(), 5);
    }

    #[test]
    fn clear_summary() {
        let mut o = ClearOptions::default();
        assert!(o.any());
        assert_eq!(o.summary(), "Cleared history, cookies and cache");
        o = ClearOptions { history: false, cookies: false, cache: false, downloads: true, range: TimeRange::AllTime };
        assert_eq!(o.summary(), "Cleared downloads list");
        o.downloads = false;
        assert!(!o.any());
    }

    #[test]
    fn sites_and_origins() {
        assert_eq!(site_of_host("www.bbc.co.uk"), "bbc.co.uk");
        assert_eq!(site_of_host(".accounts.google.com"), "google.com");
        assert_eq!(site_of_host("localhost"), "localhost");
        assert_eq!(site_of_host("192.168.1.4"), "192.168.1.4");
        assert!(cookie_in_site(".mail.google.com", "google.com"));
        assert!(!cookie_in_site("notgoogle.com", "google.com"));
        assert_eq!(
            cookie_sites([".google.com", "mail.google.com", "github.com"]),
            vec![("github.com".to_string(), 1), ("google.com".to_string(), 2)]
        );
        let origins = origins_of(["https://a.test/x", "https://a.test/y", "http://b.test:8080/", "mori://settings"]);
        assert_eq!(origins.into_iter().collect::<Vec<_>>(), ["http://b.test:8080", "https://a.test"]);
    }

    #[test]
    fn bytes() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1_500), "1.5 KB");
        assert_eq!(format_bytes(23_000_000), "23 MB");
    }

    #[test]
    fn notification_texts() {
        let (title, body) = notification_text("  New\tmessage ", "<b>Hi</b> & bye");
        assert_eq!(title, "New message");
        assert_eq!(body, "&lt;b&gt;Hi&lt;/b&gt; &amp; bye");
        let (long, _) = notification_text(&"x".repeat(500), "");
        assert_eq!(long.chars().count(), MAX_TITLE_CHARS + 1);
        assert!(long.ends_with('…'));
        assert_eq!(notification_source("https://www.slack.com/x"), "slack.com");
        assert_eq!(notification_source("file:///x"), "Mori");
    }

    #[test]
    fn bridge_messages() {
        let notify = serde_json::json!({"t": "notify", "nid": 3, "title": "Hi", "body": "there", "token": "x"});
        assert_eq!(
            BridgeMessage::parse(&notify),
            Some(BridgeMessage::Notify { nid: 3, title: "Hi".into(), body: "there".into(), tag: String::new(), silent: false })
        );
        assert_eq!(
            BridgeMessage::parse(&serde_json::json!({"t": "pip", "rid": 2, "open": true})),
            Some(BridgeMessage::Pip { rid: Some(2), open: true })
        );
        assert_eq!(
            BridgeMessage::parse(&serde_json::json!({"t": "clip-read", "rid": 9})),
            Some(BridgeMessage::ClipRead { rid: 9 })
        );
        assert_eq!(BridgeMessage::parse(&serde_json::json!({"t": "bogus"})), None);
        assert_eq!(bridge_reply_script(4, &serde_json::json!("granted")), r#"window.__moriReply && window.__moriReply(4, "granted")"#);
    }

    #[test]
    fn bridge_script_placeholders() {
        let on = page_bridge_script(true);
        assert!(on.starts_with("(() => {\n  if (true)"));
        assert!(on.contains(BRIDGE_TOKEN_PLACEHOLDER), "the engine fills the token in");
        assert!(on.contains(r#"const marker = "\u0001mori-bridge:";"#));
        assert!(!on.contains("__MARKER__"));
        assert!(page_bridge_script(false).contains("if (false)"));
    }

    #[test]
    fn bridge_script_keeps_its_token_to_itself() {
        let s = page_bridge_script(false);
        // Never set on an object page script can reach through its
        // prototype, nor serialised with a JSON.stringify read at call time.
        assert!(!s.contains("msg.token"));
        assert!(s.contains("const stringify = JSON.stringify;"));
        assert!(s.contains("Object.create(null)"));
        assert!(!s.contains("JSON.stringify(m"));
        // Picture-in-Picture wants a user gesture, as in Chrome.
        assert!(s.contains("activation.isActive"));
    }
}
