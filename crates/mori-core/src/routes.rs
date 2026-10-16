//! Air Traffic Control (`routes.json`): host rules that route pages into a
//! Space when they commit.

use std::path::Path;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    paths,
    util::{host_matches, strip_www},
};

pub const ROUTES_FILE: &str = "routes.json";

/// "Pages on `pattern` (or its subdomains) open in Space `context_id`."
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Route {
    pub id: Uuid,
    pub pattern: String,
    pub context_id: Uuid,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

/// Normalise free-form input to a bare host pattern: lowercased, host taken
/// from a URL if one was given, path/query/port removed, leading `www.`
/// stripped. Returns `None` unless the result contains a `.`.
pub fn normalize_pattern(input: &str) -> Option<String> {
    let mut text = input.trim().to_lowercase();
    if text.contains("://")
        && let Some(host) = url::Url::parse(&text)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
    {
        text = host;
    }
    if let Some(end) = text.find(['/', '?', '#']) {
        text.truncate(end);
    }
    if let Some(colon) = text.find(':') {
        text.truncate(colon);
    }
    let text = strip_www(&text).trim_matches('.').to_string();
    (!text.is_empty() && text.contains('.') && !text.contains(char::is_whitespace)).then_some(text)
}

/// Persistent routing rules, newest first.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RouteStore {
    pub routes: Vec<Route>,
}

impl RouteStore {
    pub fn load(dir: &Path) -> RouteStore {
        paths::read_json(&dir.join(ROUTES_FILE)).unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) -> anyhow::Result<()> {
        paths::write_json_atomic(&dir.join(ROUTES_FILE), self)
    }

    pub fn list(&self) -> &[Route] {
        &self.routes
    }

    /// Add a rule at the top, replacing any rule with the same normalised
    /// pattern. `None` if the pattern is invalid.
    pub fn add(&mut self, pattern: &str, context_id: Uuid) -> Option<&Route> {
        let pattern = normalize_pattern(pattern)?;
        self.routes.retain(|r| r.pattern != pattern);
        self.routes.insert(
            0,
            Route {
                id: Uuid::new_v4(),
                pattern,
                context_id,
                enabled: true,
            },
        );
        self.routes.first()
    }

    pub fn remove(&mut self, id: Uuid) -> bool {
        let before = self.routes.len();
        self.routes.retain(|r| r.id != id);
        self.routes.len() != before
    }

    pub fn set_enabled(&mut self, id: Uuid, enabled: bool) -> bool {
        match self.routes.iter_mut().find(|r| r.id == id) {
            Some(route) => {
                route.enabled = enabled;
                true
            }
            None => false,
        }
    }

    /// Drop every rule targeting a Space that is being deleted.
    pub fn remove_for_context(&mut self, context_id: Uuid) -> bool {
        let before = self.routes.len();
        self.routes.retain(|r| r.context_id != context_id);
        self.routes.len() != before
    }

    /// The most specific (longest-pattern) enabled rule for `host`.
    pub fn match_host(&self, host: &str) -> Option<&Route> {
        let host = host.trim().to_lowercase();
        let host = strip_www(&host);
        self.routes
            .iter()
            .filter(|r| r.enabled && host_matches(host, &r.pattern))
            .fold(None, |best: Option<&Route>, r| match best {
                Some(b) if b.pattern.len() >= r.pattern.len() => Some(b),
                _ => Some(r),
            })
    }

    /// [`RouteStore::match_host`] for a full URL.
    pub fn match_url(&self, url: &str) -> Option<&Route> {
        self.match_host(&crate::util::host_of(url)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::test_support::TempDir;

    #[test]
    fn normalization() {
        assert_eq!(
            normalize_pattern("https://WWW.GitHub.com/foo").as_deref(),
            Some("github.com")
        );
        assert_eq!(
            normalize_pattern(" docs.rs:443/x ").as_deref(),
            Some("docs.rs")
        );
        assert_eq!(
            normalize_pattern("www.example.co.uk").as_deref(),
            Some("example.co.uk")
        );
        assert_eq!(normalize_pattern("localhost"), None);
        assert_eq!(normalize_pattern(""), None);
        assert_eq!(normalize_pattern("www."), None);
    }

    #[test]
    fn longest_pattern_wins() {
        let work = Uuid::new_v4();
        let personal = Uuid::new_v4();
        let mut rs = RouteStore::default();
        rs.add("google.com", personal).unwrap();
        let docs_id = rs.add("docs.google.com", work).unwrap().id;
        assert_eq!(
            rs.match_host("docs.google.com").map(|r| r.context_id),
            Some(work)
        );
        assert_eq!(
            rs.match_host("a.docs.google.com").map(|r| r.context_id),
            Some(work)
        );
        assert_eq!(
            rs.match_host("WWW.google.com").map(|r| r.context_id),
            Some(personal)
        );
        assert_eq!(
            rs.match_host("mail.google.com").map(|r| r.context_id),
            Some(personal)
        );
        assert!(rs.match_host("notgoogle.com").is_none());
        assert_eq!(
            rs.match_url("https://docs.google.com/d/1")
                .map(|r| r.context_id),
            Some(work)
        );

        rs.set_enabled(docs_id, false);
        assert_eq!(
            rs.match_host("docs.google.com").map(|r| r.context_id),
            Some(personal)
        );
    }

    #[test]
    fn replace_remove_persist() {
        let tmp = TempDir::new();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let mut rs = RouteStore::default();
        rs.add("x.com", a);
        rs.add("y.com", a);
        rs.add("https://www.x.com/", b);
        assert_eq!(rs.list().len(), 2);
        assert_eq!(rs.list()[0].pattern, "x.com");
        assert_eq!(rs.list()[0].context_id, b);
        assert!(rs.add("nope", a).is_none());
        rs.save(tmp.path()).unwrap();
        assert_eq!(RouteStore::load(tmp.path()), rs);
        assert!(rs.remove_for_context(a));
        assert_eq!(rs.list().len(), 1);
        let id = rs.list()[0].id;
        assert!(rs.remove(id));
        assert!(rs.list().is_empty());
    }
}
