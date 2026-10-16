//! Page scripts evaluated in web content (reader view, media control, page
//! text extraction). Each script is a self-contained expression whose value
//! is the result returned to the caller (e.g. via DevTools `Runtime.evaluate`
//! with `returnByValue`).

/// Replace the document with a clean, themed reading view of the main
/// article. Evaluates to `true` when applied (or already on), `false` when no
/// article with enough text was found. Colours follow
/// `prefers-color-scheme`. Turning reader off is a plain reload.
pub const READER_SCRIPT: &str = r#"(() => {
  try {
    if (window.__moriReaderOn) return true;
    const pick = () => {
      const direct = document.querySelector('article')
        || document.querySelector('[role=main]')
        || document.querySelector('main');
      if (direct && (direct.innerText || '').length > 250) return direct;
      let best = null, bestLen = 0;
      document.querySelectorAll('div,section,article').forEach((el) => {
        const ps = el.querySelectorAll('p');
        if (ps.length < 3) return;
        let len = 0;
        ps.forEach((p) => { len += (p.innerText || '').length; });
        if (len > bestLen) { bestLen = len; best = el; }
      });
      return best;
    };
    const node = pick();
    if (!node || (node.innerText || '').length < 250) return false;
    const h1 = document.querySelector('h1');
    const title = (h1 && h1.innerText) || document.title || '';
    const dark = matchMedia('(prefers-color-scheme: dark)').matches;
    const article = node.cloneNode(true);
    article.querySelectorAll(
      'script,style,iframe,noscript,aside,nav,footer,header,form,button,svg,[role=navigation],[aria-hidden=true]'
    ).forEach((e) => e.remove());
    const bg = dark ? '#1b1b1d' : '#faf9f6';
    const fg = dark ? '#e7e7e7' : '#1a1a1a';
    const muted = dark ? '#9a9a9e' : '#6b6b6b';
    const link = dark ? '#8ab4f8' : '#1a64d6';
    document.documentElement.innerHTML = '<head><meta charset="utf-8"></head><body></body>';
    const style = document.createElement('style');
    style.textContent = `
      html,body{margin:0;background:${bg};}
      .mori-reader{max-width:720px;margin:0 auto;padding:72px 24px 140px;
        font:19px/1.72 ui-serif,Georgia,'Times New Roman',serif;color:${fg};
        -webkit-font-smoothing:antialiased;}
      .mori-reader h1{font:600 34px/1.22 system-ui,-apple-system,'Segoe UI',sans-serif;
        margin:0 0 28px;letter-spacing:-0.01em;}
      .mori-reader h2,.mori-reader h3,.mori-reader h4{
        font-family:system-ui,-apple-system,'Segoe UI',sans-serif;line-height:1.3;margin:1.7em 0 .5em;}
      .mori-reader p{margin:0 0 1.15em;}
      .mori-reader img,.mori-reader figure img{max-width:100%;height:auto;border-radius:10px;margin:1.2em 0;}
      .mori-reader figure{margin:1.4em 0;}
      .mori-reader figcaption{color:${muted};font-size:14px;text-align:center;margin-top:.5em;}
      .mori-reader a{color:${link};text-decoration:none;}
      .mori-reader a:hover{text-decoration:underline;}
      .mori-reader blockquote{margin:1.4em 0;padding:.2em 0 .2em 1.1em;
        border-left:3px solid ${muted};color:${muted};}
      .mori-reader pre,.mori-reader code{font-family:ui-monospace,SFMono-Regular,monospace;font-size:15px;}
      .mori-reader pre{background:${dark ? '#111' : '#f0ede6'};padding:14px;border-radius:8px;overflow:auto;}
      ::selection{background:${dark ? '#3a3a3c' : '#dcd6c8'};}
    `;
    document.head.appendChild(style);
    const wrap = document.createElement('div');
    wrap.className = 'mori-reader';
    const h = document.createElement('h1');
    h.textContent = title;
    wrap.appendChild(h);
    wrap.appendChild(article);
    document.body.appendChild(wrap);
    window.scrollTo(0, 0);
    window.__moriReaderOn = true;
    return true;
  } catch (e) { return false; }
})()"#;

/// Shared JS helpers for picking the page's primary `<video>`/`<audio>`:
/// a real playback session (playing with sound, or one the user interacted
/// with), preferring playing elements, then the largest video. Muted
/// YouTube hover previews are ignored.
macro_rules! media_pick_js {
    () => {
        r#"
  const closestElement = (target, selector) => {
    for (let n = target; n; n = n.parentNode || (n.host || null)) {
      if (n.nodeType === 1 && n.matches && n.matches(selector)) return n;
    }
    return null;
  };
  const hasAudibleTrack = (el) => !el.muted && (typeof el.volume !== 'number' || el.volume > 0);
  const isYouTubePreview = (el) => {
    const host = location.hostname.replace(/^www\./, '');
    if (host !== 'youtube.com' && host !== 'm.youtube.com') return false;
    if (!el.muted) return false;
    return !closestElement(el, '#movie_player, ytd-player, #shorts-player');
  };
  const eligible = (el) => {
    if (!el || isYouTubePreview(el)) return false;
    if (el.__moriMediaEligible || el.__moriMediaUserSelected) return true;
    return !el.paused && hasAudibleTrack(el);
  };
  const pick = () => {
    const els = Array.from(document.querySelectorAll('video,audio')).filter((m) => {
      if (!(m.currentSrc || m.src)) return false;
      const ok = eligible(m);
      if (ok) m.__moriMediaEligible = true;
      return ok;
    });
    if (!els.length) return null;
    els.sort((a, b) => {
      const ap = a.paused ? 0 : 1, bp = b.paused ? 0 : 1;
      if (ap !== bp) return bp - ap;
      return (b.videoWidth || 0) * (b.videoHeight || 0) - (a.videoWidth || 0) * (a.videoHeight || 0);
    });
    return els[0];
  };
"#
    };
}
const MEDIA_PICK_JS: &str = media_pick_js!();

/// Evaluates to a JSON-able snapshot of the page's primary media:
/// `{hasMedia, playing, title, artist, artwork, position, duration, muted,
/// isVideo, inPiP, canPiP}` (only `{hasMedia: false}` when there is none).
/// Title/artist/artwork come from `navigator.mediaSession.metadata` when set,
/// else the document title and the site's host.
pub const MEDIA_STATE_SCRIPT: &str = concat!(
    "(() => {\n  try {",
    media_pick_js!(),
    r#"
    const el = pick();
    if (!el) return { hasMedia: false };
    let md = null;
    try {
      const m = navigator.mediaSession && navigator.mediaSession.metadata;
      if (m) {
        const art = (m.artwork && m.artwork.length) ? m.artwork[m.artwork.length - 1].src : '';
        md = { title: m.title || '', artist: m.artist || '', artwork: art || '' };
      }
    } catch (e) {}
    return {
      hasMedia: true,
      playing: !el.paused,
      title: (md && md.title) || document.title || '',
      artist: (md && md.artist) || location.hostname.replace(/^www\./, ''),
      artwork: (md && md.artwork) || '',
      position: el.currentTime || 0,
      duration: isFinite(el.duration) ? el.duration : 0,
      muted: !!el.muted,
      isVideo: el.tagName === 'VIDEO',
      inPiP: document.pictureInPictureElement === el,
      canPiP: el.tagName === 'VIDEO' && !!document.pictureInPictureEnabled
    };
  } catch (e) { return { hasMedia: false }; }
})()"#
);

/// Installs listeners that mark media the user interacts with as eligible
/// (so a muted or just-started player still counts). Optional; inject once
/// per document before polling [`MEDIA_STATE_SCRIPT`].
pub const MEDIA_GESTURE_AGENT_SCRIPT: &str = r#"(() => {
  if (window.__moriMediaInstalled) return;
  window.__moriMediaInstalled = true;
  const mediaFrom = (target) => {
    for (let n = target; n; n = n.parentNode || (n.host || null)) {
      if (n.nodeType === 1 && (n.tagName === 'VIDEO' || n.tagName === 'AUDIO')) return n;
      if (n.nodeType === 1 && n.matches && n.matches('#movie_player, ytd-player, #shorts-player')) {
        return n.querySelector('video,audio');
      }
    }
    return null;
  };
  ['pointerdown', 'click', 'keydown', 'touchstart'].forEach((ev) => {
    document.addEventListener(ev, (e) => {
      const el = mediaFrom(e.target);
      if (el) el.__moriMediaUserSelected = true;
    }, true);
  });
})()"#;

/// A control action for the page's primary media element.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MediaCommand {
    TogglePlay,
    /// Relative seek in seconds (clamped at 0).
    SeekBy(f64),
    /// Absolute seek in seconds.
    SeekTo(f64),
    ToggleMute,
}

impl MediaCommand {
    fn action_and_value(self) -> (&'static str, f64) {
        match self {
            MediaCommand::TogglePlay => ("toggle", 0.0),
            MediaCommand::SeekBy(v) => ("seekBy", v),
            MediaCommand::SeekTo(v) => ("seek", v),
            MediaCommand::ToggleMute => ("mute", 0.0),
        }
    }
}

/// A fixed script performing `cmd` on the primary media element. Run it with
/// a user gesture so gated APIs (play) are allowed. Non-finite values
/// become 0.
pub fn media_command_script(cmd: MediaCommand) -> String {
    let (action, value) = cmd.action_and_value();
    let value = if value.is_finite() { value } else { 0.0 };
    format!(
        r#"(() => {{
  const action = "{action}";
  const value = {value:?};
{MEDIA_PICK_JS}
  const el = pick();
  if (!el) return;
  switch (action) {{
    case 'toggle':
      if (el.paused) {{ const p = el.play && el.play(); if (p && p.catch) p.catch(() => {{}}); }}
      else if (el.pause) el.pause();
      break;
    case 'seek':
      el.currentTime = Math.max(0, value);
      break;
    case 'seekBy':
      el.currentTime = Math.max(0, (el.currentTime || 0) + value);
      break;
    case 'mute':
      el.muted = !el.muted;
      break;
  }}
}})()"#
    )
}

/// Evaluates to `{title, url, text}` with the page's visible text
/// (`document.body.innerText`) truncated to 20 000 characters.
pub const PAGE_TEXT_SCRIPT: &str = r#"(() => {
  let text = '';
  try { text = (document.body && document.body.innerText) || ''; } catch (e) {}
  const chars = Array.from(text);
  if (chars.length > 20000) text = chars.slice(0, 20000).join('');
  return { title: document.title || '', url: location.href, text };
})()"#;

/// Evaluates to where the Picture-in-Picture video is drawn in the page's
/// frame, in device pixels: `{x, y, width, height, playing}` (the picture
/// itself, without the element's letterbox bars), or `null` without a
/// video. With `scroll`, a video partly off screen is scrolled into view
/// first.
pub fn pip_rect_script(scroll: bool) -> String {
    format!(
        r#"(() => {{
  try {{{MEDIA_PICK_JS}
    const el = document.pictureInPictureElement || pick();
    if (!el || el.tagName !== 'VIDEO') return null;
    let r = el.getBoundingClientRect();
    if ({scroll} && (r.top < 0 || r.bottom > innerHeight || r.left < 0 || r.right > innerWidth)) {{
      el.scrollIntoView({{ block: 'center', inline: 'center' }});
      r = el.getBoundingClientRect();
    }}
    let x = r.left, y = r.top, w = r.width, h = r.height;
    const fit = getComputedStyle(el).objectFit;
    if (el.videoWidth && el.videoHeight && w > 0 && h > 0 && fit !== 'cover' && fit !== 'fill') {{
      const aspect = el.videoWidth / el.videoHeight;
      if (w / h > aspect) {{ const nw = h * aspect; x += (w - nw) / 2; w = nw; }}
      else {{ const nh = w / aspect; y += (h - nh) / 2; h = nh; }}
    }}
    const k = window.devicePixelRatio || 1;
    return {{ x: x * k, y: y * k, width: w * k, height: h * k, playing: !el.paused }};
  }} catch (e) {{ return null; }}
}})()"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pip_rect() {
        assert!(pip_rect_script(true).contains("if (true &&"));
        assert!(pip_rect_script(false).contains("if (false &&"));
        assert!(pip_rect_script(false).contains("const pick = () =>"));
    }

    #[test]
    fn media_commands() {
        let s = media_command_script(MediaCommand::SeekBy(-10.0));
        assert!(s.contains(r#"const action = "seekBy";"#));
        assert!(s.contains("const value = -10.0;"));
        let s = media_command_script(MediaCommand::SeekTo(f64::NAN));
        assert!(s.contains("const value = 0.0;"));
        assert!(media_command_script(MediaCommand::ToggleMute).contains(r#""mute""#));
    }

    #[test]
    fn scripts_are_expressions() {
        for s in [
            READER_SCRIPT,
            MEDIA_STATE_SCRIPT,
            PAGE_TEXT_SCRIPT,
            MEDIA_GESTURE_AGENT_SCRIPT,
        ] {
            assert!(s.starts_with("(() => {"));
            assert!(s.ends_with("})()"));
        }
        assert!(MEDIA_STATE_SCRIPT.contains("mediaSession"));
        assert!(READER_SCRIPT.contains("prefers-color-scheme: dark"));
    }
}

/// Installs a per-document recorder of what the page copies, for bridging
/// Chromium's internal clipboard to the system one. It records copies made
/// through the `copy` event (Ctrl+C, `execCommand('copy')`, selection) and
/// through `navigator.clipboard.writeText`/`write`. Idempotent.
pub const CLIPBOARD_BRIDGE_SCRIPT: &str = r#"(() => {
  if (window.__moriClip) return;
  const clip = (window.__moriClip = { seq: 0, text: '' });
  const record = (t) => {
    if (typeof t !== 'string' || !t) return;
    clip.seq += 1;
    clip.text = t;
  };
  // Bubble phase on window runs after the page's own copy handlers.
  window.addEventListener('copy', (e) => {
    let t = '';
    try { t = e.clipboardData ? e.clipboardData.getData('text/plain') : ''; } catch (_) {}
    if (!t) {
      const a = document.activeElement;
      if (a && (a.tagName === 'TEXTAREA' || (a.tagName === 'INPUT' && /^(text|search|url|tel|email)$/i.test(a.type || 'text')))) {
        try { t = a.value.substring(a.selectionStart, a.selectionEnd); } catch (_) {}
      }
      if (!t) t = String(document.getSelection() || '');
    }
    record(t);
  });
  window.addEventListener('cut', (e) => {
    const a = document.activeElement;
    let t = String(document.getSelection() || '');
    try { if (a && 'value' in a) t = a.value.substring(a.selectionStart, a.selectionEnd) || t; } catch (_) {}
    record(t);
  }, true);
  const c = navigator.clipboard;
  if (c && c.writeText) {
    const writeText = c.writeText.bind(c);
    c.writeText = function (t) { record(String(t)); return writeText(t); };
  }
  if (c && c.write) {
    const write = c.write.bind(c);
    c.write = async function (items) {
      try {
        for (const item of items) {
          if (item.types.includes('text/plain')) record(await (await item.getType('text/plain')).text());
        }
      } catch (_) {}
      return write(items);
    };
  }
})()"#;

/// Evaluates to `{seq, text}` of the last copy recorded by
/// [`CLIPBOARD_BRIDGE_SCRIPT`], or `null` if the recorder isn't installed.
pub const CLIPBOARD_POLL_SCRIPT: &str =
    "(() => window.__moriClip ? { seq: window.__moriClip.seq, text: window.__moriClip.text } : null)()";

/// Puts `text` on Chromium's own clipboard (as a copy event the page never
/// sees), so a following paste delivers it with genuine paste events.
/// Evaluates to `true` on success.
pub fn clipboard_seed_script(text: &str) -> String {
    let literal = serde_json::to_string(text).unwrap_or_else(|_| "\"\"".into());
    format!(
        r#"(() => {{
  const text = {literal};
  let done = false;
  const seed = (e) => {{
    e.clipboardData.setData('text/plain', text);
    e.preventDefault();
    e.stopImmediatePropagation();
    done = true;
  }};
  window.addEventListener('copy', seed, {{ capture: true, once: true }});
  const clip = window.__moriClip;
  const before = clip ? clip.seq : 0;
  try {{ document.execCommand('copy'); }} finally {{
    window.removeEventListener('copy', seed, {{ capture: true }});
  }}
  if (clip) {{ clip.seq = before; }}
  return done;
}})()"#
    )
}

#[cfg(test)]
mod clipboard_tests {
    use super::clipboard_seed_script;

    #[test]
    fn seed_script_escapes_text() {
        let s = clipboard_seed_script("a\"b</script>\n");
        assert!(s.contains(r#"const text = "a\"b</script>\n";"#));
    }
}
