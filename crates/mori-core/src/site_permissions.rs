//! Remembered permission decisions per site (`site_permissions.json`).
//!
//! When a page asks for a permission (camera, location, …) and the user
//! allows or blocks it, the answer is kept for the page's origin so the
//! prompt isn't shown again; the site-info popover lists and changes them.

use std::{collections::BTreeMap, path::Path};

use serde::{Deserialize, Serialize};

use crate::paths;

pub const SITE_PERMISSIONS_FILE: &str = "site_permissions.json";

/// A remembered answer to a permission prompt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Decision {
    Allow,
    Block,
}

/// Decisions keyed by origin (`scheme://host[:port]`), then by permission
/// name (e.g. `"camera"`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SitePermissionStore {
    sites: BTreeMap<String, BTreeMap<String, Decision>>,
}

/// The origin decisions are stored under: `scheme://host[:port]` for http(s)
/// URLs (default ports omitted, host lowercased), `None` otherwise.
pub fn origin_of(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url.trim()).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    let host = parsed.host_str()?.to_ascii_lowercase();
    Some(match parsed.port() {
        Some(port) => format!("{}://{host}:{port}", parsed.scheme()),
        None => format!("{}://{host}", parsed.scheme()),
    })
}

impl SitePermissionStore {
    pub fn load(dir: &Path) -> SitePermissionStore {
        paths::read_json(&dir.join(SITE_PERMISSIONS_FILE)).unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) -> anyhow::Result<()> {
        paths::write_json_atomic(&dir.join(SITE_PERMISSIONS_FILE), self)
    }

    /// The remembered decision for `permission` on the page at `url`.
    pub fn get(&self, url: &str, permission: &str) -> Option<Decision> {
        let origin = origin_of(url)?;
        self.sites.get(&origin)?.get(permission).copied()
    }

    /// Remember `decision` for `permission` on `url`'s origin. Returns
    /// whether anything changed (non-web URLs are ignored).
    pub fn set(&mut self, url: &str, permission: &str, decision: Decision) -> bool {
        let Some(origin) = origin_of(url) else { return false };
        self.sites
            .entry(origin)
            .or_default()
            .insert(permission.to_string(), decision)
            != Some(decision)
    }

    /// Forget the decision for `permission` on `url`'s origin.
    pub fn forget(&mut self, url: &str, permission: &str) -> bool {
        let Some(origin) = origin_of(url) else { return false };
        let Some(site) = self.sites.get_mut(&origin) else { return false };
        let removed = site.remove(permission).is_some();
        if site.is_empty() {
            self.sites.remove(&origin);
        }
        removed
    }

    /// Forget every decision for `url`'s origin.
    pub fn forget_site(&mut self, url: &str) -> bool {
        origin_of(url).is_some_and(|origin| self.sites.remove(&origin).is_some())
    }

    /// Decisions for `url`'s origin, by permission name.
    pub fn for_site(&self, url: &str) -> Vec<(String, Decision)> {
        origin_of(url)
            .and_then(|origin| self.sites.get(&origin))
            .map(|site| site.iter().map(|(k, d)| (k.clone(), *d)).collect())
            .unwrap_or_default()
    }

    /// The answer for a prompt asking for all of `permissions` at once:
    /// `Some(false)` if any is blocked, `Some(true)` if all are allowed,
    /// `None` (ask the user) otherwise or when nothing was asked.
    pub fn answer(&self, url: &str, permissions: &[&str]) -> Option<bool> {
        if permissions.is_empty() {
            return None;
        }
        let decisions: Vec<Option<Decision>> = permissions.iter().map(|p| self.get(url, p)).collect();
        if decisions.contains(&Some(Decision::Block)) {
            Some(false)
        } else if decisions.iter().all(|d| *d == Some(Decision::Allow)) {
            Some(true)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::test_support::TempDir;

    #[test]
    fn origins() {
        assert_eq!(origin_of("https://Example.com/a?b#c").as_deref(), Some("https://example.com"));
        assert_eq!(origin_of("https://example.com:443/").as_deref(), Some("https://example.com"));
        assert_eq!(origin_of("http://localhost:3000/x").as_deref(), Some("http://localhost:3000"));
        assert_eq!(origin_of("file:///tmp/a.html"), None);
        assert_eq!(origin_of("mori://settings"), None);
        assert_eq!(origin_of("nonsense"), None);
    }

    #[test]
    fn remember_answer_and_forget() {
        let mut s = SitePermissionStore::default();
        let page = "https://meet.example.com/room/1";
        assert_eq!(s.answer(page, &["camera", "microphone"]), None);
        assert!(s.set(page, "camera", Decision::Allow));
        assert!(!s.set("https://meet.example.com/other", "camera", Decision::Allow), "same origin, no change");
        assert_eq!(s.answer(page, &["camera"]), Some(true));
        assert_eq!(s.answer(page, &["camera", "microphone"]), None, "microphone still unknown");
        s.set(page, "microphone", Decision::Block);
        assert_eq!(s.answer(page, &["camera", "microphone"]), Some(false), "any block wins");
        assert_eq!(s.answer(page, &[]), None);
        assert_eq!(s.answer("https://other.example.com", &["camera"]), None, "per origin");
        assert_eq!(
            s.for_site(page),
            [("camera".to_string(), Decision::Allow), ("microphone".to_string(), Decision::Block)]
        );
        assert!(s.forget(page, "camera"));
        assert!(!s.forget(page, "camera"));
        assert!(s.forget_site(page));
        assert!(s.for_site(page).is_empty());
        assert!(!s.set("file:///x", "camera", Decision::Allow));
    }

    #[test]
    fn roundtrip() {
        let tmp = TempDir::new();
        let mut s = SitePermissionStore::default();
        s.set("https://a.test", "geolocation", Decision::Block);
        s.save(tmp.path()).unwrap();
        assert_eq!(SitePermissionStore::load(tmp.path()), s);
        let raw = std::fs::read_to_string(tmp.path().join(SITE_PERMISSIONS_FILE)).unwrap();
        assert!(raw.contains(r#""https://a.test""#) && raw.contains(r#""block""#));
    }
}
