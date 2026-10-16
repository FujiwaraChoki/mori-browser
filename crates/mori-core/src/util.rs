//! Small URL helpers shared across modules.

use url::Url;

/// The lowercased host of `url`, or `None` when it has no (non-empty) host.
pub fn host_of(url: &str) -> Option<String> {
    let parsed = Url::parse(url.trim()).ok()?;
    let host = parsed.host_str()?.to_ascii_lowercase();
    (!host.is_empty()).then_some(host)
}

/// The origin of `url` as the web serialises it, port included when it
/// isn't the scheme's default: `http://localhost:8795` for
/// `http://localhost:8795/x`. `None` for URLs without one (`about:`,
/// `data:`, `file:`).
pub fn origin_of(url: &str) -> Option<String> {
    let origin = Url::parse(url.trim()).ok()?.origin();
    origin.is_tuple().then(|| origin.ascii_serialization())
}

/// `host` without a single leading `www.`.
pub fn strip_www(host: &str) -> &str {
    host.strip_prefix("www.").unwrap_or(host)
}

/// True when `host` equals `domain` or is a subdomain of it.
pub fn host_matches(host: &str, domain: &str) -> bool {
    !domain.is_empty()
        && (host == domain
            || (host.len() > domain.len()
                && host.ends_with(domain)
                && host.as_bytes()[host.len() - domain.len() - 1] == b'.'))
}

/// The site of `host` for "same site" checks: its last two labels, or three
/// under a short second-level suffix like `co.uk` or `com.au`. An
/// approximation of the public-suffix rules that is good enough to tell
/// `mail.google.com` and `docs.google.com` apart from `example.com`.
/// IP addresses are their own site.
pub fn site_of(host: &str) -> &str {
    let host = host.trim_end_matches('.');
    if host.parse::<std::net::IpAddr>().is_ok() || host.starts_with('[') {
        return host;
    }
    let labels: Vec<&str> = host.split('.').collect();
    let n = labels.len();
    if n <= 2 {
        return host;
    }
    let short_suffix = labels[n - 1].len() == 2 && labels[n - 2].len() <= 3;
    let keep = if short_suffix { 3 } else { 2 };
    let start: usize = labels[..n - keep].iter().map(|l| l.len() + 1).sum();
    &host[start..]
}

/// Whether two URLs belong to the same site (see [`site_of`]).
pub fn same_site(a: &str, b: &str) -> bool {
    match (host_of(a), host_of(b)) {
        (Some(a), Some(b)) => site_of(&a) == site_of(&b),
        _ => false,
    }
}

/// Truncate to at most `max` characters (not bytes).
pub fn truncate_chars(s: &str, max: usize) -> &str {
    match s.char_indices().nth(max) {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts() {
        assert_eq!(
            host_of("https://WWW.Example.com:8080/x").as_deref(),
            Some("www.example.com")
        );
        assert_eq!(host_of("about:blank"), None);
        assert_eq!(host_of("not a url"), None);
        assert_eq!(strip_www("www.a.com"), "a.com");
        assert!(host_matches("a.example.com", "example.com"));
        assert!(host_matches("example.com", "example.com"));
        assert!(!host_matches("badexample.com", "example.com"));
        assert_eq!(truncate_chars("héllo", 2), "hé");
    }

    #[test]
    fn origins() {
        assert_eq!(origin_of("http://localhost:8795/x?y").as_deref(), Some("http://localhost:8795"));
        assert_eq!(origin_of("https://Example.com:443/a").as_deref(), Some("https://example.com"));
        assert_eq!(origin_of("https://example.com:8443/").as_deref(), Some("https://example.com:8443"));
        assert_eq!(origin_of("about:blank"), None);
        assert_eq!(origin_of("data:text/plain,x"), None);
    }

    #[test]
    fn sites() {
        assert_eq!(site_of("mail.google.com"), "google.com");
        assert_eq!(site_of("google.com"), "google.com");
        assert_eq!(site_of("a.b.example.co.uk"), "example.co.uk");
        assert_eq!(site_of("www.abc.net.au"), "abc.net.au");
        assert_eq!(site_of("localhost"), "localhost");
        assert_eq!(site_of("192.168.1.10"), "192.168.1.10");
        assert!(same_site("https://mail.google.com/x", "https://accounts.google.com/y"));
        assert!(!same_site("https://x.com/home", "https://github.com/"));
        assert!(!same_site("about:blank", "about:blank"));
    }
}
