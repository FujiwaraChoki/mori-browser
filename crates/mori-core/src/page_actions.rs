//! Page scripts behind chrome actions: context-menu media controls and the
//! modifier-click hook that opens links in Peek.

/// A context-menu action on a `<video>` / `<audio>` element.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaAction {
    TogglePlay,
    ToggleMute,
    ToggleLoop,
    ToggleControls,
}

/// Script applying `action` to the main-frame media element playing `src`
/// (matched against `currentSrc` and `src`). Evaluates to whether one was
/// found.
pub fn media_action_script(src: &str, action: MediaAction) -> String {
    let literal = serde_json::to_string(src).unwrap_or_else(|_| "\"\"".into());
    let body = match action {
        MediaAction::TogglePlay => "if (m.paused) { m.play(); } else { m.pause(); }",
        MediaAction::ToggleMute => "m.muted = !m.muted;",
        MediaAction::ToggleLoop => "m.loop = !m.loop;",
        MediaAction::ToggleControls => "m.controls = !m.controls;",
    };
    format!(
        "(() => {{ const src = {literal}; \
         const m = [...document.querySelectorAll('video, audio')].find(e => e.currentSrc === src || e.src === src); \
         if (!m) return false; {body} return true; }})()"
    )
}

/// Console marker the Peek hook logs; never shown to users.
const PEEK_MARKER: &str = "\u{2063}mori:peek ";

/// Installed in every page: Alt+click on a link opens it in Peek (like Zen's
/// Glance) instead of Chromium's "download link". The click is cancelled and
/// the URL reported through a console marker, see [`peek_request`].
pub const PEEK_LINK_SCRIPT: &str = r#"(() => {
  if (window.__moriPeekLinks) return;
  window.__moriPeekLinks = true;
  document.addEventListener('click', (e) => {
    if (!e.isTrusted || e.button !== 0 || !e.altKey || e.ctrlKey || e.metaKey || e.shiftKey) return;
    const a = e.target && e.target.closest ? e.target.closest('a[href]') : null;
    if (!a || !/^https?:/i.test(a.href)) return;
    e.preventDefault();
    e.stopImmediatePropagation();
    console.debug('\u2063mori:peek ' + a.href);
  }, true);
})()"#;

/// The URL a [`PEEK_LINK_SCRIPT`] console message asks to peek, if the
/// message is one and the URL is http(s).
pub fn peek_request(message: &str) -> Option<String> {
    web_url(message.strip_prefix(PEEK_MARKER)?)
}

fn web_url(text: &str) -> Option<String> {
    let url = text.trim();
    let parsed = url::Url::parse(url).ok()?;
    matches!(parsed.scheme(), "http" | "https").then(|| url.to_string())
}

/// Console marker of [`PINNED_PEEK_SCRIPT`]; never shown to users.
const PINNED_PEEK_MARKER: &str = "\u{2063}mori:pinned-peek ";

/// Installed in pinned tabs (Arc): a plain click on a link to another site
/// opens it in Peek instead of taking the pinned tab away from its site.
/// "Another site" follows [`crate::util::site_of`]. The click is cancelled
/// and the URL reported through a console marker, see
/// [`pinned_peek_request`]. Modified clicks, downloads and links within the
/// site behave as usual; [`PINNED_PEEK_OFF_SCRIPT`] turns it off.
pub const PINNED_PEEK_SCRIPT: &str = r#"(() => {
  window.__moriPinnedPeek = true;
  if (window.__moriPinnedPeekInstalled) return;
  window.__moriPinnedPeekInstalled = true;
  const site = (host) => {
    host = host.replace(/\.$/, '');
    if (/^[\d.]+$/.test(host) || host.startsWith('[')) return host;
    const l = host.split('.');
    if (l.length <= 2) return host;
    const short = l[l.length - 1].length === 2 && l[l.length - 2].length <= 3;
    return l.slice(-(short ? 3 : 2)).join('.');
  };
  document.addEventListener('click', (e) => {
    if (!window.__moriPinnedPeek || !e.isTrusted || e.button !== 0 || e.defaultPrevented) return;
    if (e.altKey || e.ctrlKey || e.metaKey || e.shiftKey) return;
    const a = e.target && e.target.closest ? e.target.closest('a[href]') : null;
    if (!a || a.hasAttribute('download') || !/^https?:/i.test(a.href)) return;
    if (site(new URL(a.href).hostname) === site(location.hostname)) return;
    e.preventDefault();
    e.stopImmediatePropagation();
    console.debug('\u2063mori:pinned-peek ' + a.href);
  }, true);
})()"#;

/// Turns [`PINNED_PEEK_SCRIPT`] off in a page whose tab was unpinned.
pub const PINNED_PEEK_OFF_SCRIPT: &str = "window.__moriPinnedPeek = false";

/// The URL a [`PINNED_PEEK_SCRIPT`] console message asks to peek, if the
/// message is one and the URL is http(s).
pub fn pinned_peek_request(message: &str) -> Option<String> {
    web_url(message.strip_prefix(PINNED_PEEK_MARKER)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_scripts_quote_the_source() {
        let s = media_action_script("https://a.test/v.webm?x=\"1\"", MediaAction::ToggleMute);
        assert!(s.contains(r#"const src = "https://a.test/v.webm?x=\"1\"";"#));
        assert!(s.contains("m.muted = !m.muted;"));
        assert!(media_action_script("x", MediaAction::TogglePlay).contains("m.play()"));
        assert!(media_action_script("x", MediaAction::ToggleLoop).contains("m.loop = !m.loop"));
        assert!(media_action_script("x", MediaAction::ToggleControls).contains("m.controls = !m.controls"));
    }

    #[test]
    fn peek_requests() {
        assert_eq!(
            peek_request("\u{2063}mori:peek https://example.com/a?b").as_deref(),
            Some("https://example.com/a?b")
        );
        assert_eq!(peek_request("\u{2063}mori:peek javascript:alert(1)"), None);
        assert_eq!(peek_request("\u{2063}mori:peek file:///etc/passwd"), None);
        assert_eq!(peek_request("mori:peek https://example.com"), None);
        assert_eq!(peek_request("hello"), None);
        // The script logs exactly the marker the parser expects.
        assert!(PEEK_LINK_SCRIPT.contains("'\\u2063mori:peek '"));
    }

    #[test]
    fn pinned_peek_requests() {
        assert_eq!(
            pinned_peek_request("\u{2063}mori:pinned-peek https://github.com/").as_deref(),
            Some("https://github.com/")
        );
        assert_eq!(pinned_peek_request("\u{2063}mori:peek https://github.com/"), None);
        assert_eq!(peek_request("\u{2063}mori:pinned-peek https://github.com/"), None);
        assert_eq!(pinned_peek_request("\u{2063}mori:pinned-peek data:text/html,x"), None);
        assert!(PINNED_PEEK_SCRIPT.contains("'\\u2063mori:pinned-peek '"));
    }
}
