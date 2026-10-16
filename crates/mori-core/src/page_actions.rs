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
    let url = message.strip_prefix(PEEK_MARKER)?.trim();
    let parsed = url::Url::parse(url).ok()?;
    matches!(parsed.scheme(), "http" | "https").then(|| url.to_string())
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
}
