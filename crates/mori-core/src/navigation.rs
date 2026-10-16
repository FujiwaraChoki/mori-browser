//! Typed addresses that Mori upgraded to https: hosts that only speak plain
//! http (routers, NAS boxes, LAN dev servers) and the http fallback when an
//! upgraded load fails, like Chrome's HTTPS-Upgrades.

use std::net::IpAddr;

/// Hosts that are reachable over plain http in practice: loopback,
/// private, link-local and carrier-grade-NAT addresses, single-label names
/// and LAN-only suffixes.
pub fn prefers_http(host: &str) -> bool {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = bare.parse::<IpAddr>() {
        return match ip {
            IpAddr::V4(v4) => {
                let [a, b, ..] = v4.octets();
                v4.is_loopback()
                    || v4.is_private()
                    || v4.is_link_local()
                    || v4.is_unspecified()
                    || (a == 100 && (64..128).contains(&b))
            }
            IpAddr::V6(v6) => {
                let first = v6.segments()[0];
                v6.is_loopback()
                    || v6.is_unspecified()
                    || (first & 0xfe00) == 0xfc00
                    || (first & 0xffc0) == 0xfe80
            }
        };
    }
    if host.is_empty() {
        return false;
    }
    if !host.contains('.') {
        return true;
    }
    const LAN_SUFFIXES: [&str; 5] = [".localhost", ".local", ".lan", ".internal", ".home.arpa"];
    LAN_SUFFIXES.iter().any(|s| host.ends_with(s))
}

/// For an address typed without a scheme that resolved to `https://…`:
/// the URL to actually open. LAN-style hosts get `http://` straight away.
pub fn typed_url(input: &str, resolved: &str) -> String {
    if has_scheme(input) {
        return resolved.to_string();
    }
    match resolved.strip_prefix("https://") {
        Some(rest) if crate::util::host_of(resolved).is_some_and(|h| prefers_http(&h)) => {
            format!("http://{rest}")
        }
        _ => resolved.to_string(),
    }
}

/// Whether the address `input` (not a search) was upgraded to https by
/// Mori rather than typed that way, so a failed load may retry over http.
pub fn is_upgraded(input: &str, url: &str) -> bool {
    !has_scheme(input) && url.starts_with("https://")
}

/// The plain-http URL to retry when an upgraded https `url` failed with the
/// Chromium net error `code`, or `None` when http would not help (DNS
/// failures, offline, aborted loads).
pub fn http_fallback(url: &str, code: i32) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    retry_over_http(code).then(|| format!("http://{rest}"))
}

/// Net errors where the server may well answer over plain http.
fn retry_over_http(code: i32) -> bool {
    matches!(
        code,
        -7      // TIMED_OUT
        | -15   // SOCKET_NOT_CONNECTED
        | -100  // CONNECTION_CLOSED
        | -101  // CONNECTION_RESET
        | -102  // CONNECTION_REFUSED
        | -104  // CONNECTION_FAILED
        | -107  // SSL_PROTOCOL_ERROR
        | -109  // ADDRESS_UNREACHABLE
        | -113  // SSL_VERSION_OR_CIPHER_MISMATCH
        | -118  // CONNECTION_TIMED_OUT
        | -324  // EMPTY_RESPONSE
    ) || (-299..=-200).contains(&code) // certificate errors
}

fn has_scheme(input: &str) -> bool {
    let input = input.trim();
    match input.find("://") {
        Some(i) => input[..i].chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c)),
        None => {
            let lower = input.to_ascii_lowercase();
            ["about:", "data:", "mori:", "chrome:", "view-source:", "file:", "javascript:"]
                .iter()
                .any(|p| lower.starts_with(p))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lan_hosts_prefer_http() {
        for host in [
            "192.168.1.1",
            "10.0.0.5",
            "172.16.3.4",
            "127.0.0.1",
            "169.254.1.1",
            "100.100.1.1",
            "[::1]",
            "fd12::1",
            "fe80::1",
            "nas",
            "router.local",
            "printer.lan",
            "app.localhost",
            "box.home.arpa",
        ] {
            assert!(prefers_http(host), "{host}");
        }
        for host in ["8.8.8.8", "example.com", "neverssl.com", "2001:4860::8888", "100.200.1.1", ""] {
            assert!(!prefers_http(host), "{host}");
        }
    }

    #[test]
    fn typed_lan_addresses_open_over_http() {
        assert_eq!(typed_url("192.168.1.1", "https://192.168.1.1/"), "http://192.168.1.1/");
        assert_eq!(typed_url("nas.local:5000/x", "https://nas.local:5000/x"), "http://nas.local:5000/x");
        assert_eq!(typed_url("example.com", "https://example.com/"), "https://example.com/");
        // An explicit scheme is respected.
        assert_eq!(typed_url("https://192.168.1.1", "https://192.168.1.1/"), "https://192.168.1.1/");
    }

    #[test]
    fn upgraded_loads_fall_back_on_connection_and_tls_errors() {
        assert!(is_upgraded("neverssl.com", "https://neverssl.com/"));
        assert!(!is_upgraded("https://neverssl.com", "https://neverssl.com/"));
        assert!(!is_upgraded("http://neverssl.com", "http://neverssl.com/"));
        assert_eq!(
            http_fallback("https://neverssl.com/", -324).as_deref(),
            Some("http://neverssl.com/")
        );
        assert!(http_fallback("https://a.test/", -202).is_some(), "bad certificate");
        assert!(http_fallback("https://a.test/", -102).is_some(), "refused");
        assert!(http_fallback("https://a.test/", -105).is_none(), "DNS failure");
        assert!(http_fallback("https://a.test/", -106).is_none(), "offline");
        assert!(http_fallback("http://a.test/", -324).is_none(), "already http");
    }
}
