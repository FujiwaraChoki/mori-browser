//! Small URL helpers shared across modules.

use url::Url;

/// The lowercased host of `url`, or `None` when it has no (non-empty) host.
pub fn host_of(url: &str) -> Option<String> {
    let parsed = Url::parse(url.trim()).ok()?;
    let host = parsed.host_str()?.to_ascii_lowercase();
    (!host.is_empty()).then_some(host)
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
}
