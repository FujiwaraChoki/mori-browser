//! Rules for the questions pages ask the user: JavaScript dialogs,
//! credential prompts and permission requests.
//!
//! Every pending prompt belongs to one page (its owner). The chrome shows the
//! oldest prompt of the page on screen and must answer exactly that prompt,
//! by id, never simply the oldest prompt overall: a background page's
//! request must not be granted by a click aimed at the foreground page.

use std::collections::HashMap;

use url::Url;

/// Pending prompts in arrival order, each tagged with the page that asked.
#[derive(Debug)]
pub struct PromptQueue<O, T> {
    items: Vec<(O, T)>,
}

impl<O, T> Default for PromptQueue<O, T> {
    fn default() -> Self {
        Self { items: Vec::new() }
    }
}

impl<O: Copy + PartialEq, T> PromptQueue<O, T> {
    pub fn push(&mut self, owner: O, item: T) {
        self.items.push((owner, item));
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The prompt `owner` shows now: its oldest.
    pub fn front(&self, owner: O) -> Option<&T> {
        self.items.iter().find(|(o, _)| *o == owner).map(|(_, t)| t)
    }

    pub fn has(&self, owner: O) -> bool {
        self.front(owner).is_some()
    }

    /// Take the prompt matching `pred` (normally its id), wherever it is.
    pub fn take(&mut self, pred: impl Fn(&T) -> bool) -> Option<(O, T)> {
        let i = self.items.iter().position(|(_, t)| pred(t))?;
        Some(self.items.remove(i))
    }

    /// Drop every prompt of `owner` (page closed or navigated away),
    /// returning them so the caller can cancel each.
    pub fn take_owner(&mut self, owner: O) -> Vec<T> {
        let (gone, kept) = std::mem::take(&mut self.items)
            .into_iter()
            .partition(|(o, _)| *o == owner);
        self.items = kept;
        gone.into_iter().map(|(_, t)| t).collect()
    }

    /// Hand every prompt of `from` to `to`, e.g. when a peeked page becomes
    /// a tab.
    pub fn reassign(&mut self, from: O, to: O) {
        for (owner, _) in &mut self.items {
            if *owner == from {
                *owner = to;
            }
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &(O, T)> {
        self.items.iter()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Block,
}

/// The user's Allow/Block answers per site for the current session, so a
/// page that asks again (or reloads) is not prompted over and over. Not
/// persisted: a mistaken answer is forgotten on restart.
#[derive(Debug)]
pub struct SitePermissions<K> {
    decisions: HashMap<(String, K), Decision>,
}

impl<K> Default for SitePermissions<K> {
    fn default() -> Self {
        Self { decisions: HashMap::new() }
    }
}

/// `scheme://host[:port]`, lowercased; the unit permissions are granted to.
pub fn origin_key(url: &str) -> Option<String> {
    let parsed = Url::parse(url.trim()).ok()?;
    let origin = parsed.origin();
    origin.is_tuple().then(|| origin.ascii_serialization())
}

impl<K: Copy + Eq + std::hash::Hash> SitePermissions<K> {
    /// The remembered answer for a request of `kinds` from `origin`: Block
    /// if any kind was blocked, Allow if every kind was allowed, otherwise
    /// `None` (ask).
    pub fn decide(&self, origin: &str, kinds: &[K]) -> Option<Decision> {
        let origin = origin_key(origin)?;
        let mut all_allowed = !kinds.is_empty();
        for kind in kinds {
            match self.decisions.get(&(origin.clone(), *kind)) {
                Some(Decision::Block) => return Some(Decision::Block),
                Some(Decision::Allow) => {}
                None => all_allowed = false,
            }
        }
        all_allowed.then_some(Decision::Allow)
    }

    pub fn record(&mut self, origin: &str, kinds: &[K], decision: Decision) {
        let Some(origin) = origin_key(origin) else { return };
        for kind in kinds {
            self.decisions.insert((origin.clone(), *kind), decision);
        }
    }
}

/// What a TLS certificate error means, for the interstitial page:
/// `(Chromium's error name, one-sentence reason)`.
pub fn certificate_error_text(code: i32) -> (&'static str, &'static str) {
    match code {
        -200 => (
            "NET::ERR_CERT_COMMON_NAME_INVALID",
            "The certificate was issued for a different site.",
        ),
        -201 => (
            "NET::ERR_CERT_DATE_INVALID",
            "The certificate has expired or is not valid yet. Check that your computer's clock is right.",
        ),
        -202 => (
            "NET::ERR_CERT_AUTHORITY_INVALID",
            "The certificate is not issued by an authority your computer trusts. It may be self-signed.",
        ),
        -206 => ("NET::ERR_CERT_REVOKED", "The certificate has been revoked by its issuer."),
        -208 => (
            "NET::ERR_CERT_WEAK_SIGNATURE_ALGORITHM",
            "The certificate is signed with a weak algorithm.",
        ),
        -211 => ("NET::ERR_CERT_WEAK_KEY", "The certificate uses a weak key."),
        -213 => (
            "NET::ERR_CERT_VALIDITY_TOO_LONG",
            "The certificate is valid for longer than allowed.",
        ),
        -217 => (
            "NET::ERR_CERT_KNOWN_INTERCEPTION_BLOCKED",
            "The connection is being intercepted by software known to be unsafe.",
        ),
        _ => ("NET::ERR_CERT_INVALID", "The site's security certificate is not valid."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    struct Prompt(u32);

    #[test]
    fn shows_and_answers_the_owners_prompt() {
        let mut q = PromptQueue::default();
        q.push('a', Prompt(1)); // background tab asked first
        q.push('b', Prompt(2));
        q.push('b', Prompt(3));
        // The visible page shows its own oldest prompt...
        assert_eq!(q.front('b'), Some(&Prompt(2)));
        // ...and answering it by id leaves the background request alone.
        assert_eq!(q.take(|p| p.0 == 2), Some(('b', Prompt(2))));
        assert_eq!(q.front('a'), Some(&Prompt(1)));
        assert_eq!(q.front('b'), Some(&Prompt(3)));
        assert!(q.take(|p| p.0 == 99).is_none());
    }

    #[test]
    fn closing_a_page_drops_only_its_prompts() {
        let mut q = PromptQueue::default();
        q.push(1, Prompt(1));
        q.push(2, Prompt(2));
        q.push(1, Prompt(3));
        assert_eq!(q.take_owner(1), vec![Prompt(1), Prompt(3)]);
        assert!(!q.has(1));
        assert!(q.has(2));
        assert!(!q.is_empty());
        q.reassign(2, 3);
        assert!(!q.has(2));
        assert_eq!(q.front(3), Some(&Prompt(2)));
    }

    #[test]
    fn certificate_errors_have_names() {
        assert_eq!(certificate_error_text(-202).0, "NET::ERR_CERT_AUTHORITY_INVALID");
        assert_eq!(certificate_error_text(-201).0, "NET::ERR_CERT_DATE_INVALID");
        assert_eq!(certificate_error_text(-999).0, "NET::ERR_CERT_INVALID");
    }

    #[test]
    fn origins_normalise() {
        assert_eq!(
            origin_key("HTTPS://Maps.Example.com/a?b").as_deref(),
            Some("https://maps.example.com")
        );
        assert_eq!(
            origin_key("http://127.0.0.1:8080/").as_deref(),
            Some("http://127.0.0.1:8080")
        );
        assert_eq!(origin_key("about:blank"), None);
        assert_eq!(origin_key("garbage"), None);
    }

    #[test]
    fn remembers_answers_per_origin_and_kind() {
        let mut p = SitePermissions::default();
        assert_eq!(p.decide("https://a.com", &["camera"]), None);
        p.record("https://a.com/call", &["camera", "mic"], Decision::Allow);
        assert_eq!(p.decide("https://a.com/other", &["camera"]), Some(Decision::Allow));
        assert_eq!(p.decide("https://a.com", &["camera", "mic"]), Some(Decision::Allow));
        // A new kind still asks; another site still asks.
        assert_eq!(p.decide("https://a.com", &["camera", "geo"]), None);
        assert_eq!(p.decide("https://b.com", &["camera"]), None);
        // Any blocked kind blocks the whole request.
        p.record("https://a.com", &["geo"], Decision::Block);
        assert_eq!(p.decide("https://a.com", &["camera", "geo"]), Some(Decision::Block));
        assert_eq!(p.decide("https://a.com", &[]), None);
        assert_eq!(p.decide("not a url", &["camera"]), None);
    }
}
