//! Per-site Boosts (`boosts.json`): custom CSS/JS plus "zapped" element
//! selectors, matched by host suffix. Also the page scripts that apply a
//! boost and drive click-to-zap.

use std::path::Path;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    paths,
    util::{host_matches, strip_www},
};

pub const BOOSTS_FILE: &str = "boosts.json";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Boost {
    pub id: Uuid,
    /// Lowercased host with `www.` stripped; also covers its subdomains.
    pub host: String,
    #[serde(default)]
    pub css: String,
    #[serde(default)]
    pub js: String,
    #[serde(default)]
    pub zapped_selectors: Vec<String>,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// UNIX seconds.
    #[serde(default)]
    pub updated_at: f64,
}

fn yes() -> bool {
    true
}

impl Boost {
    /// A blank, enabled boost for `host` (normalised).
    pub fn new(host: &str) -> Boost {
        Boost {
            id: Uuid::new_v4(),
            host: normalize_host(host),
            css: String::new(),
            js: String::new(),
            zapped_selectors: Vec::new(),
            enabled: true,
            updated_at: paths::now(),
        }
    }

    /// True when there is nothing to inject.
    pub fn is_empty(&self) -> bool {
        self.css.trim().is_empty() && self.js.trim().is_empty() && self.zapped_selectors.is_empty()
    }
}

/// Lowercase, trim and strip a leading `www.`.
pub fn normalize_host(host: &str) -> String {
    let lower = host.trim().to_lowercase();
    strip_www(&lower).to_string()
}

/// The host a boost is keyed on for `url` (`None` for host-less URLs).
pub fn boost_host_for_url(url: &str) -> Option<String> {
    crate::util::host_of(url).map(|h| normalize_host(&h))
}

/// Persistent boosts, newest first.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BoostStore {
    pub boosts: Vec<Boost>,
}

impl BoostStore {
    pub fn load(dir: &Path) -> BoostStore {
        paths::read_json(&dir.join(BOOSTS_FILE)).unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) -> anyhow::Result<()> {
        paths::write_json_atomic(&dir.join(BOOSTS_FILE), self)
    }

    pub fn list(&self) -> &[Boost] {
        &self.boosts
    }

    /// The first *enabled* boost covering `host` — the one to inject.
    pub fn for_host(&self, host: &str) -> Option<&Boost> {
        let host = normalize_host(host);
        self.boosts
            .iter()
            .find(|b| b.enabled && host_matches(&host, &b.host))
    }

    /// The first boost covering `host`, enabled or not (for the editor).
    pub fn find_any(&self, host: &str) -> Option<&Boost> {
        let host = normalize_host(host);
        self.boosts.iter().find(|b| host_matches(&host, &b.host))
    }

    /// The boost keyed exactly on `host`, if any.
    pub fn exact(&self, host: &str) -> Option<&Boost> {
        let host = normalize_host(host);
        self.boosts.iter().find(|b| b.host == host)
    }

    /// Insert or replace (matched by id, else by host). New boosts go first;
    /// existing ones keep their position. `updated_at` is refreshed and empty
    /// boosts are deleted.
    pub fn upsert(&mut self, mut boost: Boost) {
        boost.host = normalize_host(&boost.host);
        boost.updated_at = paths::now();
        let existing = self
            .boosts
            .iter()
            .position(|b| b.id == boost.id)
            .or_else(|| self.boosts.iter().position(|b| b.host == boost.host));
        match existing {
            Some(idx) => self.boosts[idx] = boost,
            None => self.boosts.insert(0, boost),
        }
        self.boosts.retain(|b| !b.is_empty());
    }

    /// Append zapped selectors (deduplicated) to the boost for `host`.
    pub fn add_zaps(&mut self, host: &str, selectors: &[String]) {
        if selectors.is_empty() {
            return;
        }
        let mut boost = self
            .exact(host)
            .cloned()
            .unwrap_or_else(|| Boost::new(host));
        for sel in selectors {
            if !sel.is_empty() && !boost.zapped_selectors.contains(sel) {
                boost.zapped_selectors.push(sel.clone());
            }
        }
        self.upsert(boost);
    }

    /// Forget all zapped selectors for `host`.
    pub fn clear_zaps(&mut self, host: &str) {
        if let Some(mut boost) = self.exact(host).cloned() {
            boost.zapped_selectors.clear();
            self.upsert(boost);
        }
    }

    pub fn set_enabled(&mut self, id: Uuid, enabled: bool) -> bool {
        match self.boosts.iter_mut().find(|b| b.id == id) {
            Some(b) => {
                b.enabled = enabled;
                b.updated_at = paths::now();
                true
            }
            None => false,
        }
    }

    pub fn remove(&mut self, id: Uuid) -> Option<Boost> {
        let idx = self.boosts.iter().position(|b| b.id == id)?;
        Some(self.boosts.remove(idx))
    }
}

/// Encode `s` as a JavaScript string literal (quotes included). JSON string
/// syntax is valid JS; `<`, U+2028 and U+2029 are additionally escaped so the
/// literal is safe in any embedding context.
pub fn js_string(s: &str) -> String {
    let json = serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into());
    json.replace('<', "\\u003c")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

/// An idempotent script applying `boost`: (re)writes
/// `<style id="__mori_boost_style">` with the zap rules plus custom CSS, and
/// runs the custom JS once per document (guarded by `window.__moriBoostJSDone`).
pub fn injection_script(boost: &Boost) -> String {
    let mut css = String::new();
    if !boost.zapped_selectors.is_empty() {
        css.push_str(&boost.zapped_selectors.join(",\n"));
        css.push_str(" { display: none !important; }\n");
    }
    css.push_str(&boost.css);
    format!(
        r#"(() => {{
  const css = {css};
  const js = {js};
  let style = document.getElementById('__mori_boost_style');
  if (!style) {{
    style = document.createElement('style');
    style.id = '__mori_boost_style';
    (document.head || document.documentElement).appendChild(style);
  }}
  style.textContent = css;
  if (js && !window.__moriBoostJSDone) {{
    window.__moriBoostJSDone = true;
    try {{ (new Function(js))(); }} catch (e) {{ console.error('Mori boost JS error', e); }}
  }}
}})();"#,
        css = js_string(&css),
        js = js_string(&boost.js),
    )
}

/// Arms click-to-zap: a red hover highlight and a capturing click handler
/// that hides the clicked element and records a selector in
/// `window.__moriZapped`. Idempotent.
pub const ZAP_START_SCRIPT: &str = r##"(() => {
  if (window.__moriZapActive) return;
  window.__moriZapActive = true;
  window.__moriZapped = window.__moriZapped || [];
  const esc = (s) => (window.CSS && CSS.escape) ? CSS.escape(s) : s;
  const selectorFor = (el) => {
    if (!el || el.nodeType !== 1) return "";
    if (el.id) return "#" + esc(el.id);
    const parts = [];
    let node = el;
    while (node && node.nodeType === 1 && parts.length < 6 && node !== document.body) {
      let part = node.localName || "*";
      if (node.classList && node.classList.length) {
        part += "." + Array.from(node.classList).slice(0, 2).map(esc).join(".");
      }
      const parent = node.parentElement;
      if (parent) {
        const sibs = Array.from(parent.children).filter((c) => c.localName === node.localName);
        if (sibs.length > 1) part += `:nth-of-type(${sibs.indexOf(node) + 1})`;
      }
      parts.unshift(part);
      node = parent;
    }
    return parts.join(" > ");
  };
  const hl = document.createElement('div');
  hl.id = '__mori_zap_hl';
  hl.style.cssText = 'position:fixed;z-index:2147483647;pointer-events:none;background:rgba(255,64,64,0.22);border:2px solid rgba(255,40,40,0.95);border-radius:4px;box-shadow:0 0 0 9999px rgba(0,0,0,0.04);transition:left .04s,top .04s,width .04s,height .04s;left:-9999px;top:-9999px;';
  document.documentElement.appendChild(hl);
  const move = (e) => {
    const el = document.elementFromPoint(e.clientX, e.clientY);
    if (!el || el === hl) return;
    window.__moriZapTarget = el;
    const r = el.getBoundingClientRect();
    hl.style.left = r.left + 'px';
    hl.style.top = r.top + 'px';
    hl.style.width = r.width + 'px';
    hl.style.height = r.height + 'px';
  };
  const click = (e) => {
    e.preventDefault();
    e.stopPropagation();
    const el = window.__moriZapTarget;
    if (!el) return;
    const sel = selectorFor(el);
    if (sel) {
      window.__moriZapped.push(sel);
      el.style.setProperty('display', 'none', 'important');
    }
    window.__moriZapTarget = null;
    hl.style.left = '-9999px';
    hl.style.top = '-9999px';
  };
  window.__moriZapMove = move;
  window.__moriZapClick = click;
  document.addEventListener('mousemove', move, true);
  document.addEventListener('click', click, true);
})();"##;

/// Tears zap mode down and evaluates to the array of recorded selectors
/// (an array of strings, not a JSON string).
pub const ZAP_FINISH_SCRIPT: &str = r#"(() => {
  const zapped = (window.__moriZapped || []).slice();
  if (window.__moriZapMove) document.removeEventListener('mousemove', window.__moriZapMove, true);
  if (window.__moriZapClick) document.removeEventListener('click', window.__moriZapClick, true);
  window.__moriZapMove = null;
  window.__moriZapClick = null;
  window.__moriZapTarget = null;
  const hl = document.getElementById('__mori_zap_hl');
  if (hl && hl.remove) hl.remove();
  window.__moriZapActive = false;
  window.__moriZapped = [];
  return zapped;
})()"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::test_support::TempDir;

    fn boost(host: &str, css: &str) -> Boost {
        Boost {
            css: css.into(),
            ..Boost::new(host)
        }
    }

    #[test]
    fn host_matching() {
        let mut s = BoostStore::default();
        s.upsert(boost("www.YouTube.com", "a{}"));
        assert_eq!(s.list()[0].host, "youtube.com");
        assert!(s.for_host("youtube.com").is_some());
        assert!(s.for_host("www.youtube.com").is_some());
        assert!(s.for_host("m.youtube.com").is_some());
        assert!(s.for_host("notyoutube.com").is_none());
        let id = s.list()[0].id;
        s.set_enabled(id, false);
        assert!(s.for_host("youtube.com").is_none());
        assert!(s.find_any("youtube.com").is_some());
        assert_eq!(
            boost_host_for_url("https://www.youtube.com/watch").as_deref(),
            Some("youtube.com")
        );
    }

    #[test]
    fn upsert_order_and_empty_removal() {
        let tmp = TempDir::new();
        let mut s = BoostStore::default();
        s.upsert(boost("a.com", "x{}"));
        s.upsert(boost("b.com", "y{}"));
        assert_eq!(s.list()[0].host, "b.com");
        // Same host, different id → replaces in place.
        s.upsert(boost("a.com", "z{}"));
        assert_eq!(s.list().len(), 2);
        assert_eq!(s.exact("a.com").unwrap().css, "z{}");
        s.add_zaps("a.com", &["#ad".into(), "#ad".into(), ".banner".into()]);
        assert_eq!(
            s.exact("a.com").unwrap().zapped_selectors,
            vec!["#ad", ".banner"]
        );
        s.add_zaps("new.com", &["#x".into()]);
        assert_eq!(s.list()[0].host, "new.com");
        s.clear_zaps("new.com");
        assert!(s.exact("new.com").is_none(), "emptied boost is deleted");
        s.save(tmp.path()).unwrap();
        assert_eq!(BoostStore::load(tmp.path()), s);
        let id = s.list()[0].id;
        assert!(s.remove(id).is_some());
        assert_eq!(s.list().len(), 1);
    }

    #[test]
    fn script_escaping() {
        let mut b = boost(
            "a.com",
            "body { content: \"</style>\\\" ' `${x}`\"; }\u{2028}",
        );
        b.js = "alert('hi');\n</script>".into();
        b.zapped_selectors = vec!["#a".into(), "div.b > p:nth-of-type(2)".into()];
        let script = injection_script(&b);
        assert!(!script.contains("</style>"));
        assert!(!script.contains("</script>"));
        assert!(!script.contains('\u{2028}'));
        assert!(script.contains("__mori_boost_style"));
        assert!(script.contains("__moriBoostJSDone"));
        // The embedded literal decodes back to the exact CSS.
        let line = script
            .lines()
            .find(|l| l.trim_start().starts_with("const css = "))
            .unwrap();
        let literal = line
            .trim()
            .trim_start_matches("const css = ")
            .trim_end_matches(';');
        let decoded: String = serde_json::from_str(literal).unwrap();
        assert_eq!(
            decoded,
            format!(
                "#a,\ndiv.b > p:nth-of-type(2) {{ display: none !important; }}\n{}",
                b.css
            )
        );
        assert_eq!(js_string("a\"b"), r#""a\"b""#);
    }
}
