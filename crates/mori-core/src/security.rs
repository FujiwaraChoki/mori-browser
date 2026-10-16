//! What the address bar says about a page's connection.
//!
//! The scheme alone is not enough: an `https` page whose certificate failed
//! (shown behind the "connection isn't private" page, or loaded after the
//! user chose to continue) must not get a lock.

use std::collections::HashSet;

use crate::util::host_of;

/// The connection state shown by the address bar and site info.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Security {
    /// `https` with a valid certificate.
    Secure,
    /// Plain `http`.
    NotSecure,
    /// `https` with a certificate error, pending or overridden.
    CertificateError,
    /// Browser pages, files, and the empty state: nothing to say.
    Neutral,
}

/// Whether a Chromium net error code is a certificate error
/// (`ERR_CERT_*`, -200 to -299).
pub fn is_certificate_error_code(code: i32) -> bool {
    (-299..=-200).contains(&code)
}

/// Whether Chromium certificate status bits (`net::CertStatus`) hold a real
/// error. Revocation-check trouble is minor (Chrome still shows a lock), and
/// the high bits are informational (EV, revocation checking enabled, ...).
pub fn cert_status_is_error(bits: u32) -> bool {
    const ALL_ERRORS: u32 = 0xFF00_FFFF;
    const MINOR: u32 = 16 | 32; // NO_REVOCATION_MECHANISM | UNABLE_TO_CHECK_REVOCATION
    bits & ALL_ERRORS & !MINOR != 0
}

/// Hosts whose bad certificate the user chose to accept. Chromium remembers
/// that choice for the rest of the session and then loads the host without
/// asking again, so the chrome has to remember it too. Not saved.
#[derive(Default, Debug)]
pub struct CertificateExceptions {
    hosts: HashSet<String>,
}

impl CertificateExceptions {
    /// The user continued to `url` despite its certificate.
    pub fn allow(&mut self, url: &str) {
        if let Some(host) = host_of(url) {
            self.hosts.insert(host);
        }
    }

    /// Whether `url`'s host was let through with a bad certificate.
    pub fn covers(&self, url: &str) -> bool {
        url.starts_with("https://") && host_of(url).is_some_and(|h| self.hosts.contains(&h))
    }
}

/// The state of `url`. `cert_problem` says the page has a certificate error
/// waiting for the user or showing as its error page.
pub fn classify(url: &str, cert_problem: bool, exceptions: &CertificateExceptions) -> Security {
    if url.starts_with("https://") {
        if cert_problem || exceptions.covers(url) {
            Security::CertificateError
        } else {
            Security::Secure
        }
    } else if url.starts_with("http://") {
        Security::NotSecure
    } else {
        Security::Neutral
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheme_decides_without_certificate_trouble() {
        let ex = CertificateExceptions::default();
        assert_eq!(classify("https://example.com/", false, &ex), Security::Secure);
        assert_eq!(classify("http://example.com/", false, &ex), Security::NotSecure);
        assert_eq!(classify("mori://newtab", false, &ex), Security::Neutral);
        assert_eq!(classify("file:///tmp/x.html", false, &ex), Security::Neutral);
        assert_eq!(classify("", false, &ex), Security::Neutral);
    }

    #[test]
    fn pending_or_shown_certificate_error_drops_the_lock() {
        let ex = CertificateExceptions::default();
        assert_eq!(classify("https://localhost:8766/", true, &ex), Security::CertificateError);
        // An http page has no certificate to be wrong about.
        assert_eq!(classify("http://localhost:8766/", true, &ex), Security::NotSecure);
    }

    #[test]
    fn continuing_past_a_bad_certificate_marks_the_host_for_the_session() {
        let mut ex = CertificateExceptions::default();
        ex.allow("https://localhost:8766/page2.html");
        assert_eq!(classify("https://localhost:8766/other", false, &ex), Security::CertificateError);
        // Chromium keys the decision by host, whatever the port.
        assert_eq!(classify("https://localhost/", false, &ex), Security::CertificateError);
        assert_eq!(classify("https://example.com/", false, &ex), Security::Secure);
        assert!(!ex.covers("http://localhost:8766/"));
    }

    #[test]
    fn certificate_status_bits() {
        assert!(!cert_status_is_error(0));
        assert!(cert_status_is_error(4)); // AUTHORITY_INVALID (self-signed)
        assert!(cert_status_is_error(4 | 131072)); // ... with REV_CHECKING_ENABLED
        assert!(cert_status_is_error(1 | 2)); // name and date
        assert!(!cert_status_is_error(65536)); // IS_EV
        assert!(!cert_status_is_error(131072 | 524288)); // informational bits only
        assert!(!cert_status_is_error(32)); // can't check revocation: minor
        assert!(cert_status_is_error(64)); // REVOKED
    }

    #[test]
    fn certificate_error_codes() {
        assert!(is_certificate_error_code(-202)); // ERR_CERT_AUTHORITY_INVALID
        assert!(is_certificate_error_code(-200));
        assert!(!is_certificate_error_code(-105)); // ERR_NAME_NOT_RESOLVED
        assert!(!is_certificate_error_code(-300));
        assert!(!is_certificate_error_code(0));
    }
}
