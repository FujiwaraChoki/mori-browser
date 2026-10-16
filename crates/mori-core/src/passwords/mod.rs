//! Saved logins (`passwords.json`) and the rules around them.
//!
//! Mori keeps passwords in the system keyring (the Secret Service: GNOME
//! Keyring, KWallet, KeePassXC, …). This store only holds what is safe to
//! keep in a plain file: the origin and username of each login, the id of
//! its keyring item, and when it was saved and used. Secrets only pass
//! through here as [`Secret`], which wipes itself when dropped and never
//! prints its contents.
//!
//! Logins belong to an origin (`scheme://host[:port]`). A page is offered
//! the logins of its own origin; logins of a parent or child domain, or of
//! the `http://` version of an `https://` page, are "related" and are only
//! filled after the user confirms ([`match_origin`]).

pub mod csv;
mod generator;
pub mod report;
pub mod scripts;

use std::{collections::BTreeSet, path::Path};

use serde::{Deserialize, Serialize};
use url::{Host, Url};
use uuid::Uuid;

pub use generator::generate_password;

use crate::{paths, util::host_matches};

pub const PASSWORDS_FILE: &str = "passwords.json";

/// A password (or other secret) in memory: zeroed when dropped, redacted
/// when printed.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Secret(zeroize::Zeroizing<String>);

impl Secret {
    pub fn new(value: String) -> Secret {
        Secret(zeroize::Zeroizing::new(value))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Takes the bytes the keyring returned, wiping them either way.
    pub fn from_utf8(mut bytes: Vec<u8>) -> Option<Secret> {
        let secret = std::str::from_utf8(&bytes).ok().map(|s| Secret::new(s.to_string()));
        zeroize::Zeroize::zeroize(&mut bytes);
        secret
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(***)")
    }
}

/// The origin logins are saved under: `scheme://host[:port]` for http(s)
/// URLs (host lowercased, default port omitted), `None` for anything else.
pub fn login_origin(url: &str) -> Option<String> {
    crate::site_permissions::origin_of(url)
}

/// `host[:port]` of an origin, for display.
pub fn display_origin(origin: &str) -> String {
    origin
        .split_once("://")
        .map(|(_, rest)| rest.to_string())
        .unwrap_or_else(|| origin.to_string())
}

/// How a saved login's origin relates to the page being filled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum OriginMatch {
    /// Same scheme, host and port: offered and filled directly.
    Exact,
    /// A parent or child domain, or the `http://` version of an `https://`
    /// page: offered, but only filled after the user confirms.
    Related,
}

/// Hosting domains whose subdomains belong to unrelated people, so they
/// never count as related to each other or to the parent. (Full public
/// suffixes such as `co.uk` never host a login page, so they need no entry.)
const SHARED_HOSTING: &[&str] = &[
    "appspot.com",
    "azurewebsites.net",
    "blogspot.com",
    "cloudfront.net",
    "firebaseapp.com",
    "fly.dev",
    "github.io",
    "gitlab.io",
    "glitch.me",
    "herokuapp.com",
    "netlify.app",
    "ngrok-free.app",
    "ngrok.io",
    "onrender.com",
    "pages.dev",
    "vercel.app",
    "web.app",
    "workers.dev",
];

/// Whether logins saved for `saved` may be offered on a page of `page`
/// (both origins or URLs), and how.
///
/// Related means: the same scheme and port with one host a subdomain of the
/// other (`example.com` and `www.example.com`), or a login saved over
/// `http://` offered to the `https://` page of the same host. A login saved
/// over `https://` is never offered to an `http://` page, and IP addresses
/// and shared hosting domains only ever match exactly.
pub fn match_origin(saved: &str, page: &str) -> Option<OriginMatch> {
    let (saved_origin, page_origin) = (login_origin(saved)?, login_origin(page)?);
    if saved_origin == page_origin {
        return Some(OriginMatch::Exact);
    }
    let (s, p) = (Url::parse(&saved_origin).ok()?, Url::parse(&page_origin).ok()?);
    let upgrade = s.scheme() == "http" && p.scheme() == "https";
    if upgrade {
        // Only the default ports: other ports are other services.
        if s.port().is_some() || p.port().is_some() {
            return None;
        }
    } else if s.scheme() != p.scheme() || s.port() != p.port() {
        return None;
    }
    let (Some(Host::Domain(sh)), Some(Host::Domain(ph))) = (s.host(), p.host()) else {
        return None;
    };
    if sh == ph {
        return upgrade.then_some(OriginMatch::Related);
    }
    let parent = if host_matches(ph, sh) {
        sh
    } else if host_matches(sh, ph) {
        ph
    } else {
        return None;
    };
    if parent.split('.').count() < 2 || SHARED_HOSTING.contains(&parent) {
        return None;
    }
    Some(OriginMatch::Related)
}

/// A saved login. The password itself lives in the keyring, in the item
/// tagged with `id`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Credential {
    pub id: Uuid,
    pub origin: String,
    pub username: String,
    pub created: f64,
    /// When the password last changed.
    pub updated: f64,
    /// When it was last filled or used to sign in (0 = never).
    pub last_used: f64,
    pub times_used: u32,
}

impl Default for Credential {
    fn default() -> Self {
        Credential {
            id: Uuid::nil(),
            origin: String::new(),
            username: String::new(),
            created: 0.0,
            updated: 0.0,
            last_used: 0.0,
            times_used: 0,
        }
    }
}

impl Credential {
    /// `host[:port]` of the login's origin.
    pub fn host(&self) -> String {
        display_origin(&self.origin)
    }
}

/// What submitting a login form should lead to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveTarget {
    /// A login Mori doesn't have yet: offer to save it.
    New,
    /// The saved login for this username: offer to update its password if
    /// it changed.
    Existing(Uuid),
    /// The user chose "Never" for this site, or turned saving off.
    Never,
}

/// Every saved login, the sites never to offer saving for, and whether to
/// offer saving at all.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PasswordStore {
    pub offer_to_save: bool,
    credentials: Vec<Credential>,
    never_save: BTreeSet<String>,
}

impl Default for PasswordStore {
    fn default() -> Self {
        PasswordStore {
            offer_to_save: true,
            credentials: Vec::new(),
            never_save: BTreeSet::new(),
        }
    }
}

impl PasswordStore {
    pub fn load(dir: &Path) -> PasswordStore {
        let mut store: PasswordStore = paths::read_json(&dir.join(PASSWORDS_FILE)).unwrap_or_default();
        store.credentials.retain(|c| !c.id.is_nil() && login_origin(&c.origin).is_some());
        store
    }

    pub fn save(&self, dir: &Path) -> anyhow::Result<()> {
        paths::write_json_atomic(&dir.join(PASSWORDS_FILE), self)
    }

    pub fn credentials(&self) -> &[Credential] {
        &self.credentials
    }

    pub fn get(&self, id: Uuid) -> Option<&Credential> {
        self.credentials.iter().find(|c| c.id == id)
    }

    /// The saved login for `username` on exactly `origin`.
    pub fn find(&self, origin: &str, username: &str) -> Option<&Credential> {
        let origin = login_origin(origin)?;
        self.credentials.iter().find(|c| c.origin == origin && c.username == username)
    }

    /// Logins to offer on the page at `url`: exact matches first, then
    /// related ones, each most recently used first.
    pub fn for_page(&self, url: &str) -> Vec<(&Credential, OriginMatch)> {
        let mut found: Vec<(&Credential, OriginMatch)> = self
            .credentials
            .iter()
            .filter_map(|c| match_origin(&c.origin, url).map(|m| (c, m)))
            .collect();
        found.sort_by(|(a, am), (b, bm)| {
            am.cmp(bm)
                .then(b.last_used.total_cmp(&a.last_used))
                .then_with(|| a.username.to_lowercase().cmp(&b.username.to_lowercase()))
        });
        found
    }

    /// Whether the page at `url` has any login to offer.
    pub fn has_logins_for(&self, url: &str) -> bool {
        self.credentials.iter().any(|c| match_origin(&c.origin, url).is_some())
    }

    /// Logins whose site or username contains `query` (case-insensitive),
    /// sorted by site, then username. An empty query lists everything.
    pub fn search(&self, query: &str) -> Vec<&Credential> {
        let q = query.trim().to_lowercase();
        let mut found: Vec<&Credential> = self
            .credentials
            .iter()
            .filter(|c| q.is_empty() || c.host().to_lowercase().contains(&q) || c.username.to_lowercase().contains(&q))
            .collect();
        found.sort_by(|a, b| {
            crate::util::strip_www(&a.host())
                .cmp(crate::util::strip_www(&b.host()))
                .then_with(|| a.username.to_lowercase().cmp(&b.username.to_lowercase()))
        });
        found
    }

    /// What a submitted login for `username` on `origin` should lead to.
    /// A form without a username field updates the site's only login, if
    /// it has exactly one.
    pub fn save_target(&self, origin: &str, username: &str) -> SaveTarget {
        let Some(origin) = login_origin(origin) else { return SaveTarget::Never };
        if !self.offer_to_save || self.never_save.contains(&origin) {
            return SaveTarget::Never;
        }
        if let Some(c) = self.find(&origin, username) {
            return SaveTarget::Existing(c.id);
        }
        if username.is_empty() {
            let mut site = self.credentials.iter().filter(|c| c.origin == origin);
            if let (Some(only), None) = (site.next(), site.next()) {
                return SaveTarget::Existing(only.id);
            }
        }
        SaveTarget::New
    }

    /// Record a new login whose secret is (about to be) in the keyring under
    /// `id`. Returns `None` for non-web origins.
    pub fn add(&mut self, id: Uuid, origin: &str, username: &str, now: f64) -> Option<&Credential> {
        let origin = login_origin(origin)?;
        self.credentials.retain(|c| c.id != id);
        self.credentials.push(Credential {
            id,
            origin,
            username: username.to_string(),
            created: now,
            updated: now,
            last_used: now,
            times_used: 0,
        });
        self.credentials.last()
    }

    /// The password of `id` changed.
    pub fn mark_updated(&mut self, id: Uuid, now: f64) -> bool {
        self.credentials.iter_mut().find(|c| c.id == id).map(|c| c.updated = now).is_some()
    }

    /// `id` was filled into a page or used to sign in.
    pub fn mark_used(&mut self, id: Uuid, now: f64) -> bool {
        self.credentials
            .iter_mut()
            .find(|c| c.id == id)
            .map(|c| {
                c.last_used = now;
                c.times_used = c.times_used.saturating_add(1);
            })
            .is_some()
    }

    /// Change the username of `id`. Fails (returns `false`) when another
    /// login of the same site already has that username.
    pub fn set_username(&mut self, id: Uuid, username: &str) -> bool {
        let Some(origin) = self.get(id).map(|c| c.origin.clone()) else { return false };
        if self.credentials.iter().any(|c| c.id != id && c.origin == origin && c.username == username) {
            return false;
        }
        if let Some(c) = self.credentials.iter_mut().find(|c| c.id == id) {
            c.username = username.to_string();
        }
        true
    }

    pub fn remove(&mut self, id: Uuid) -> Option<Credential> {
        let index = self.credentials.iter().position(|c| c.id == id)?;
        Some(self.credentials.remove(index))
    }

    /// Sites (origins) the user chose never to save passwords for.
    pub fn never_save(&self) -> impl Iterator<Item = &str> {
        self.never_save.iter().map(String::as_str)
    }

    /// Stop (or resume) offering to save passwords on `origin`.
    pub fn set_never_save(&mut self, origin: &str, never: bool) -> bool {
        let Some(origin) = login_origin(origin) else { return false };
        if never { self.never_save.insert(origin) } else { self.never_save.remove(&origin) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::test_support::TempDir;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn exact_origins() {
        use OriginMatch::*;
        assert_eq!(match_origin("https://example.com", "https://example.com/login?x#y"), Some(Exact));
        assert_eq!(match_origin("https://Example.com:443", "https://example.com"), Some(Exact));
        assert_eq!(match_origin("http://localhost:3000", "http://localhost:3000/a"), Some(Exact));
        assert_eq!(match_origin("http://localhost:3000", "http://localhost:3001/"), None, "port");
        assert_eq!(match_origin("https://example.com:8443", "https://example.com/"), None, "port");
        assert_eq!(match_origin("https://127.0.0.1", "https://127.0.0.1/x"), Some(Exact));
        assert_eq!(match_origin("file:///etc/passwd", "file:///etc/passwd"), None);
        assert_eq!(match_origin("https://example.com", "mori://settings"), None);
        assert_eq!(match_origin("https://example.com", "https://example.org"), None);
        assert_eq!(match_origin("https://example.com", "https://notexample.com"), None, "label boundary");
    }

    #[test]
    fn related_origins() {
        use OriginMatch::*;
        assert_eq!(match_origin("https://example.com", "https://www.example.com"), Some(Related));
        assert_eq!(match_origin("https://login.example.com", "https://example.com"), Some(Related));
        assert_eq!(match_origin("https://a.b.example.com", "https://example.com"), Some(Related));
        // Siblings are other services of the same company, or other people.
        assert_eq!(match_origin("https://mail.example.com", "https://docs.example.com"), None);
        // An http login may be offered to the https page; never the reverse.
        assert_eq!(match_origin("http://example.com", "https://example.com"), Some(Related));
        assert_eq!(match_origin("http://example.com", "https://www.example.com"), Some(Related));
        assert_eq!(match_origin("https://example.com", "http://example.com"), None);
        assert_eq!(match_origin("http://example.com:8080", "https://example.com:8080"), None);
        assert_eq!(match_origin("https://example.com", "https://www.example.com:444"), None);
        // IPs and shared hosting only match exactly.
        assert_eq!(match_origin("http://10.0.0.1", "https://10.0.0.1"), None);
        assert_eq!(match_origin("https://alice.github.io", "https://bob.github.io"), None);
        assert_eq!(match_origin("https://github.io", "https://bob.github.io"), None);
        assert_eq!(match_origin("https://x.alice.github.io", "https://alice.github.io"), Some(Related));
        assert_eq!(match_origin("https://localhost", "https://app.localhost"), None, "single label");
    }

    #[test]
    fn page_offers_are_ordered() {
        let mut s = PasswordStore::default();
        s.add(id(1), "https://www.example.com", "old", 10.0);
        s.add(id(2), "https://example.com", "bob", 20.0);
        s.add(id(3), "https://example.com", "alice", 20.0);
        s.add(id(4), "https://other.com", "zed", 30.0);
        s.mark_used(id(2), 40.0);
        let offers: Vec<(&str, OriginMatch)> =
            s.for_page("https://example.com/login").iter().map(|(c, m)| (c.username.as_str(), *m)).collect();
        assert_eq!(
            offers,
            [("bob", OriginMatch::Exact), ("alice", OriginMatch::Exact), ("old", OriginMatch::Related)]
        );
        assert!(s.has_logins_for("https://www.example.com/"));
        assert!(!s.has_logins_for("https://example.org/"));
        assert_eq!(s.get(id(2)).unwrap().times_used, 1);
    }

    #[test]
    fn save_targets() {
        let mut s = PasswordStore::default();
        assert_eq!(s.save_target("https://a.test/login", "me"), SaveTarget::New);
        s.add(id(1), "https://a.test", "me", 1.0);
        assert_eq!(s.save_target("https://a.test/x", "me"), SaveTarget::Existing(id(1)));
        assert_eq!(s.save_target("https://a.test/x", "you"), SaveTarget::New);
        assert_eq!(s.save_target("https://www.a.test/x", "me"), SaveTarget::New, "exact origin only");
        // No username field: the site's only login.
        assert_eq!(s.save_target("https://a.test", ""), SaveTarget::Existing(id(1)));
        s.add(id(2), "https://a.test", "you", 1.0);
        assert_eq!(s.save_target("https://a.test", ""), SaveTarget::New);
        assert!(s.set_never_save("https://a.test/whatever", true));
        assert_eq!(s.save_target("https://a.test", "me"), SaveTarget::Never);
        assert_eq!(s.never_save().collect::<Vec<_>>(), ["https://a.test"]);
        assert!(s.set_never_save("https://a.test", false));
        s.offer_to_save = false;
        assert_eq!(s.save_target("https://b.test", "me"), SaveTarget::Never);
        assert_eq!(s.save_target("ftp://b.test", "me"), SaveTarget::Never);
    }

    #[test]
    fn edit_and_remove() {
        let mut s = PasswordStore::default();
        assert!(s.add(id(1), "about:blank", "x", 1.0).is_none());
        s.add(id(1), "https://a.test", "me", 1.0);
        s.add(id(2), "https://a.test", "you", 1.0);
        assert!(!s.set_username(id(2), "me"), "duplicate username");
        assert!(s.set_username(id(2), "them"));
        assert!(s.mark_updated(id(2), 5.0));
        assert_eq!(s.get(id(2)).map(|c| (c.username.as_str(), c.updated)), Some(("them", 5.0)));
        assert_eq!(s.remove(id(1)).map(|c| c.username), Some("me".into()));
        assert!(s.remove(id(1)).is_none());
        assert!(!s.mark_used(id(1), 2.0));
    }

    #[test]
    fn search_by_site_or_username() {
        let mut s = PasswordStore::default();
        s.add(id(1), "https://www.github.com", "octocat", 1.0);
        s.add(id(2), "https://accounts.google.com", "me@gmail.com", 1.0);
        s.add(id(3), "https://example.com", "GitFan", 1.0);
        let names = |q: &str| s.search(q).iter().map(|c| c.username.clone()).collect::<Vec<_>>();
        assert_eq!(names(""), ["me@gmail.com", "GitFan", "octocat"]);
        assert_eq!(names("git"), ["GitFan", "octocat"]);
        assert_eq!(names("GMAIL"), ["me@gmail.com"]);
        assert!(names("nothing").is_empty());
    }

    #[test]
    fn roundtrip_drops_broken_entries() {
        let tmp = TempDir::new();
        let mut s = PasswordStore::default();
        s.add(id(1), "https://a.test", "me", 1.0);
        s.set_never_save("https://b.test", true);
        s.save(tmp.path()).unwrap();
        assert_eq!(PasswordStore::load(tmp.path()), s);
        let raw = std::fs::read_to_string(tmp.path().join(PASSWORDS_FILE)).unwrap();
        assert!(raw.contains("https://a.test") && !raw.contains("password\""));
        std::fs::write(
            tmp.path().join(PASSWORDS_FILE),
            r#"{"credentials":[{"origin":"https://a.test"},{"id":"00000000-0000-0000-0000-000000000005","origin":"https://c.test","username":"u"}]}"#,
        )
        .unwrap();
        let loaded = PasswordStore::load(tmp.path());
        assert_eq!(loaded.credentials().len(), 1, "an entry without an id is dropped");
        assert!(loaded.offer_to_save);
    }

    #[test]
    fn secrets_are_redacted() {
        let s = Secret::new("hunter2".into());
        assert_eq!(format!("{s:?}"), "Secret(***)");
        assert_eq!(s.expose(), "hunter2");
        assert_eq!(Secret::from_utf8(b"abc".to_vec()).unwrap().expose(), "abc");
        assert!(Secret::from_utf8(vec![0xff, 0xfe]).is_none());
        assert_eq!(display_origin("https://a.test:8443"), "a.test:8443");
    }
}
