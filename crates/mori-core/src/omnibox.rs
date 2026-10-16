//! Omnibox input interpretation: "is this an address or a search?", plus the
//! host/URL strings shown in the address pill.

use std::net::Ipv6Addr;

use url::Url;

use crate::settings::{SearchEngine, Settings, encode_query};

/// Schemes that are navigated to verbatim.
pub const ALLOWED_SCHEMES: [&str; 9] = [
    "http",
    "https",
    "file",
    "about",
    "mori",
    "chrome",
    "chrome-extension",
    "view-source",
    "data",
];

/// The outcome of interpreting omnibox input. Both variants carry the URL to
/// load; `Search` lets the UI label the row "Search" instead of "Open".
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolved {
    Url(String),
    Search(String),
}

impl Resolved {
    /// The URL to navigate to.
    pub fn url(&self) -> &str {
        match self {
            Resolved::Url(u) | Resolved::Search(u) => u,
        }
    }

    pub fn into_url(self) -> String {
        match self {
            Resolved::Url(u) | Resolved::Search(u) => u,
        }
    }

    pub fn is_search(&self) -> bool {
        matches!(self, Resolved::Search(_))
    }
}

/// Turn raw omnibox input into a URL or a search.
///
/// - Empty → the homepage.
/// - An allowed scheme ([`ALLOWED_SCHEMES`]) → used as-is.
/// - A scheme-less address → `https://` prepended (`http://` for localhost,
///   `*.localhost`, `::1` and `127.x`).
/// - Anything else → a search with the configured engine.
pub fn resolve(input: &str, settings: &Settings) -> Resolved {
    let text = input.trim();
    if text.is_empty() {
        return Resolved::Url(settings.homepage_url.clone());
    }
    if has_allowed_scheme(text) {
        return Resolved::Url(text.to_string());
    }
    if looks_like_web_address(text) {
        return Resolved::Url(format!("{}://{text}", default_scheme(text)));
    }
    Resolved::Search(settings.search_url(text))
}

/// True when `input` would be opened as an address rather than searched.
pub fn looks_like_address(input: &str) -> bool {
    let text = input.trim();
    !text.is_empty() && (has_allowed_scheme(text) || looks_like_web_address(text))
}

/// The host shown in the address pill: `host[:port]`; the path for `file://`;
/// `""` for `about:blank` and `mori://` pages; the URL itself when it has no
/// host (e.g. `about:version`, `data:`).
pub fn display_host(url: &str) -> String {
    if is_internal_blank(url) {
        return String::new();
    }
    let Ok(parsed) = Url::parse(url) else {
        return url.to_string();
    };
    if parsed.scheme() == "file" {
        return parsed
            .to_file_path()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| parsed.path().to_string());
    }
    match parsed.host_str() {
        Some(host) if !host.is_empty() => match parsed.port() {
            Some(port) => format!("{host}:{port}"),
            None => host.to_string(),
        },
        _ => url.to_string(),
    }
}

/// The URL shown when the address is being edited: `""` for `about:blank`
/// and `mori://` pages, otherwise the URL unchanged.
pub fn display_url(url: &str) -> String {
    if is_internal_blank(url) {
        String::new()
    } else {
        url.to_string()
    }
}

fn is_internal_blank(url: &str) -> bool {
    url == "about:blank" || url.starts_with("mori://")
}

/// The short source shown after a launcher row's title: the host without a
/// leading `www.` (`file://` paths and host-less URLs as [`display_host`]).
pub fn row_host(url: &str) -> String {
    let host = display_host(url);
    crate::util::strip_www(&host).to_string()
}

/// True when `a` and `b` are the same page: equal after parsing, ignoring
/// the fragment and a lone trailing `/` path.
pub fn same_page(a: &str, b: &str) -> bool {
    fn key(url: &str) -> String {
        match Url::parse(url.trim()) {
            Ok(mut u) => {
                u.set_fragment(None);
                let s = u.to_string();
                s.strip_suffix('/').map(str::to_string).unwrap_or(s)
            }
            Err(_) => url.trim().to_string(),
        }
    }
    !a.trim().is_empty() && key(a) == key(b)
}

// ---- Search suggestions -----------------------------------------------------

/// Most search completions the launcher shows under the first row.
pub const MAX_SUGGESTIONS: usize = 4;

/// The completion endpoint (OpenSearch suggestions JSON) for `engine`, or
/// `None` when it has none (custom engines) or the query is blank.
pub fn suggest_url(engine: SearchEngine, query: &str) -> Option<String> {
    let query = query.trim();
    if query.is_empty() {
        return None;
    }
    let q = encode_query(query);
    Some(match engine {
        SearchEngine::Google => {
            format!("https://suggestqueries.google.com/complete/search?client=firefox&ie=utf-8&oe=utf-8&q={q}")
        }
        SearchEngine::DuckDuckGo => format!("https://duckduckgo.com/ac/?type=list&q={q}"),
        SearchEngine::Bing => format!("https://api.bing.com/osjson.aspx?query={q}"),
        SearchEngine::Brave => format!("https://search.brave.com/api/suggest?q={q}"),
        SearchEngine::Custom => return None,
    })
}

/// Completions from a suggestion response: the OpenSearch form
/// `["query", ["a", "b", …]]` or DuckDuckGo's `[{"phrase": "a"}, …]`.
/// Blank entries, the query itself and case-insensitive duplicates are
/// dropped; at most `limit` are kept. Unparseable bodies yield nothing.
pub fn parse_suggestions(body: &str, query: &str, limit: usize) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };
    let Some(array) = value.as_array() else {
        return Vec::new();
    };
    let raw: Vec<&str> = match array.get(1).and_then(|v| v.as_array()) {
        Some(list) => list.iter().filter_map(|v| v.as_str()).collect(),
        None => array
            .iter()
            .filter_map(|v| v.get("phrase").and_then(|p| p.as_str()))
            .collect(),
    };
    let query = query.trim().to_lowercase();
    let mut seen = vec![query];
    let mut out = Vec::new();
    for s in raw {
        if out.len() >= limit {
            break;
        }
        let s = s.trim();
        let lower = s.to_lowercase();
        if s.is_empty() || seen.contains(&lower) {
            continue;
        }
        seen.push(lower);
        out.push(s.to_string());
    }
    out
}

/// The scheme of `text` if it starts with a syntactically valid one
/// (`ALPHA *( ALPHA / DIGIT / "+" / "-" / "." ) ":"`).
fn scheme_of(text: &str) -> Option<String> {
    let colon = text.find(':')?;
    let scheme = &text[..colon];
    let mut chars = scheme.chars();
    let first = chars.next()?;
    if !first.is_ascii_alphabetic()
        || !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    {
        return None;
    }
    Some(scheme.to_ascii_lowercase())
}

fn has_allowed_scheme(text: &str) -> bool {
    // A URL can't contain raw whitespace/control characters; such input
    // ("http://foo bar") is treated as a search, like the original.
    if text.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return false;
    }
    scheme_of(text).is_some_and(|s| ALLOWED_SCHEMES.contains(&s.as_str()))
}

/// The authority: everything before the first `/`, `?` or `#`.
fn authority_of(text: &str) -> &str {
    text.split(['/', '?', '#']).next().unwrap_or("")
}

fn looks_like_web_address(text: &str) -> bool {
    if text.chars().any(|c| c.is_whitespace() || c.is_control()) || text.contains("://") {
        return false;
    }
    let authority = authority_of(text);
    if authority.is_empty() {
        return false;
    }
    let lower = authority.to_ascii_lowercase();
    if lower == "localhost" {
        return true;
    }
    if let Some(port) = lower.strip_prefix("localhost:") {
        return is_valid_port(port);
    }

    if let Some(rest) = authority.strip_prefix('[') {
        let Some(end) = rest.find(']') else {
            return false;
        };
        let host = &rest[..end];
        let suffix = &rest[end + 1..];
        if let Some(port) = suffix.strip_prefix(':') {
            if !is_valid_port(port) {
                return false;
            }
        } else if !suffix.is_empty() {
            return false;
        }
        return is_ipv6_literal(host);
    }

    let (host, port) = match authority.split_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (authority, None),
    };
    if port.is_some_and(|p| !is_valid_port(p)) || host.is_empty() {
        return false;
    }
    is_ipv4_literal(host) || is_domain_name(host)
}

fn default_scheme(text: &str) -> &'static str {
    let authority = authority_of(text);
    let host = match authority
        .strip_prefix('[')
        .and_then(|r| r.find(']').map(|e| &r[..e]))
    {
        Some(v6) => v6,
        None => authority.split(':').next().unwrap_or(""),
    };
    if is_local_host(host) { "http" } else { "https" }
}

fn is_local_host(host: &str) -> bool {
    let lower = host.to_ascii_lowercase();
    lower == "localhost"
        || lower.ends_with(".localhost")
        || lower == "::1"
        || (is_ipv4_literal(&lower) && lower.starts_with("127."))
}

fn is_domain_name(host: &str) -> bool {
    let labels: Vec<&str> = host.split('.').collect();
    let Some(tld) = labels.last() else {
        return false;
    };
    labels.len() >= 2
        && tld.len() >= 2
        && tld.chars().any(|c| c.is_ascii_alphabetic())
        && labels.iter().all(|label| {
            (1..=63).contains(&label.len())
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
                && !label.starts_with('-')
                && !label.ends_with('-')
        })
}

/// A canonical decimal integer in `range` (no sign, no leading zeros).
fn canonical_int(raw: &str, max: u32) -> bool {
    !raw.is_empty()
        && raw.len() <= 5
        && raw.bytes().all(|b| b.is_ascii_digit())
        && (raw == "0" || !raw.starts_with('0'))
        && raw.parse::<u32>().is_ok_and(|v| v <= max)
}

fn is_ipv4_literal(host: &str) -> bool {
    let parts: Vec<&str> = host.split('.').collect();
    parts.len() == 4 && parts.iter().all(|p| canonical_int(p, 255))
}

fn is_valid_port(raw: &str) -> bool {
    canonical_int(raw, 65535)
}

fn is_ipv6_literal(host: &str) -> bool {
    host.parse::<Ipv6Addr>().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(input: &str) -> Resolved {
        resolve(input, &Settings::default())
    }

    fn url(s: &str) -> Resolved {
        Resolved::Url(s.to_string())
    }

    #[test]
    fn addresses() {
        assert_eq!(r("example.com"), url("https://example.com"));
        assert_eq!(r("  example.com  "), url("https://example.com"));
        assert_eq!(
            r("my-site.co.uk/path?x"),
            url("https://my-site.co.uk/path?x")
        );
        assert_eq!(r("example.com:8080/a"), url("https://example.com:8080/a"));
        assert_eq!(r("localhost"), url("http://localhost"));
        assert_eq!(r("localhost:3000"), url("http://localhost:3000"));
        assert_eq!(r("LocalHost:3000/x"), url("http://LocalHost:3000/x"));
        assert_eq!(r("app.localhost:5173"), url("http://app.localhost:5173"));
        assert_eq!(r("127.0.0.1:8000"), url("http://127.0.0.1:8000"));
        assert_eq!(r("1.2.3.4"), url("https://1.2.3.4"));
        assert_eq!(r("[::1]:8080"), url("http://[::1]:8080"));
        assert_eq!(r("[2001:db8::1]"), url("https://[2001:db8::1]"));
        assert_eq!(r("example.com#frag"), url("https://example.com#frag"));
    }

    #[test]
    fn schemes_kept() {
        assert_eq!(r("http://x"), url("http://x"));
        assert_eq!(r("HTTPS://Example.com"), url("HTTPS://Example.com"));
        assert_eq!(r("mori://settings"), url("mori://settings"));
        assert_eq!(r("about:blank"), url("about:blank"));
        assert_eq!(r("chrome://extensions"), url("chrome://extensions"));
        assert_eq!(
            r("view-source:https://a.com"),
            url("view-source:https://a.com")
        );
        assert_eq!(r("data:text/html,hi"), url("data:text/html,hi"));
        assert_eq!(r("file:///etc/hosts"), url("file:///etc/hosts"));
    }

    #[test]
    fn searches() {
        let search = |q: &str| Resolved::Search(Settings::default().search_url(q));
        assert_eq!(r("foo bar"), search("foo bar"));
        assert_eq!(r("a.b"), search("a.b"));
        assert_eq!(r("rust"), search("rust"));
        assert_eq!(r("1.2.3"), search("1.2.3"));
        assert_eq!(r("256.1.1.1"), search("256.1.1.1"));
        assert_eq!(r("01.2.3.4"), search("01.2.3.4"));
        assert_eq!(r("example.com:99999"), search("example.com:99999"));
        assert_eq!(r("localhost:abc"), search("localhost:abc"));
        assert_eq!(r("-bad.com"), search("-bad.com"));
        assert_eq!(r("ftp://x.com"), search("ftp://x.com"));
        assert_eq!(r("javascript:alert(1)"), search("javascript:alert(1)"));
        assert_eq!(r("http://foo bar"), search("http://foo bar"));
        assert_eq!(r("/usr/bin"), search("/usr/bin"));
        assert_eq!(r("example.123"), search("example.123"));
        assert_eq!(r("[zz]"), search("[zz]"));
        assert!(r("what is 2+2?").is_search());
    }

    #[test]
    fn empty_is_homepage() {
        assert_eq!(r(""), url("mori://newtab"));
        assert_eq!(r("   "), url("mori://newtab"));
    }

    #[test]
    fn address_predicate() {
        assert!(looks_like_address("example.com"));
        assert!(looks_like_address("mori://settings"));
        assert!(!looks_like_address("foo bar"));
        assert!(!looks_like_address(""));
    }

    #[test]
    fn display() {
        assert_eq!(
            display_host("https://www.example.com/a?b"),
            "www.example.com"
        );
        assert_eq!(display_host("http://localhost:3000/x"), "localhost:3000");
        assert_eq!(display_host("https://example.com:443/"), "example.com");
        assert_eq!(display_host("about:blank"), "");
        assert_eq!(display_host("mori://newtab"), "");
        assert_eq!(display_host("file:///home/me/a.html"), "/home/me/a.html");
        assert_eq!(display_url("mori://settings"), "");
        assert_eq!(display_url("about:blank"), "");
        assert_eq!(display_url("https://a.com/"), "https://a.com/");
        assert_eq!(row_host("https://www.wikipedia.org/wiki/X"), "wikipedia.org");
        assert_eq!(row_host("https://en.wikipedia.org/"), "en.wikipedia.org");
        assert_eq!(row_host("mori://settings"), "");
    }

    #[test]
    fn same_pages() {
        assert!(same_page("https://a.com/", "https://a.com"));
        assert!(same_page("https://a.com/x#top", "https://a.com/x"));
        assert!(same_page("HTTPS://A.com/x", "https://a.com/x"));
        assert!(!same_page("https://a.com/x", "https://a.com/y"));
        assert!(!same_page("https://a.com/?q=1", "https://a.com/?q=2"));
        assert!(!same_page("", ""));
    }

    #[test]
    fn suggestion_endpoints() {
        assert_eq!(
            suggest_url(SearchEngine::Google, "rust lang").as_deref(),
            Some("https://suggestqueries.google.com/complete/search?client=firefox&ie=utf-8&oe=utf-8&q=rust%20lang")
        );
        assert_eq!(
            suggest_url(SearchEngine::DuckDuckGo, "a&b").as_deref(),
            Some("https://duckduckgo.com/ac/?type=list&q=a%26b")
        );
        assert_eq!(
            suggest_url(SearchEngine::Bing, "x").as_deref(),
            Some("https://api.bing.com/osjson.aspx?query=x")
        );
        assert_eq!(
            suggest_url(SearchEngine::Brave, "x").as_deref(),
            Some("https://search.brave.com/api/suggest?q=x")
        );
        assert_eq!(suggest_url(SearchEngine::Custom, "x"), None);
        assert_eq!(suggest_url(SearchEngine::Google, "  "), None);
    }

    #[test]
    fn suggestion_parsing() {
        let open_search = r#"["git",["git","GitHub","git clone","github","gitlab","git rebase"]]"#;
        assert_eq!(
            parse_suggestions(open_search, "git", 4),
            ["GitHub", "git clone", "gitlab", "git rebase"]
        );
        let ddg = r#"[{"phrase":"wiki"},{"phrase":"wikipedia"},{"phrase":" "},{"phrase":"wikiHow"}]"#;
        assert_eq!(parse_suggestions(ddg, "Wiki", 4), ["wikipedia", "wikiHow"]);
        assert!(parse_suggestions("not json", "x", 4).is_empty());
        assert!(parse_suggestions(r#"{"a":1}"#, "x", 4).is_empty());
        assert!(parse_suggestions(r#"["x",["a","b"]]"#, "x", 0).is_empty());
    }
}
