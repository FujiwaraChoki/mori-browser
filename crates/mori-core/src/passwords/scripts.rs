//! Page scripts for saving and filling logins.
//!
//! Both run in Mori's isolated world ([`super::report::AGENT_WORLD`]) of the
//! main frame: page script shares the DOM with them but not their
//! variables, and cannot call the binding the agent reports through.
//! Neither runs in subframes, so a login is never offered to, or filled
//! into, a frame of another site.

use super::Secret;

/// Shared JS for finding login forms: which password fields are for the
/// current password and which choose a new one, the username field that
/// belongs to a password field, and the form (or the nearest container)
/// a field belongs to. Element methods are taken from the prototypes so a
/// form's named inputs (`<input name="querySelectorAll">`) can't shadow
/// them.
macro_rules! form_helpers_js {
    () => {
        r#"
  const USERISH = /user|e-?mail|login|account|ident|phone|mobile|handle|member|customer/i;
  const NEWISH = /new|confirm|repeat|retype|again|verify|create|register|sign-?up|join/i;
  const CURRENTISH = /current|old|existing/i;
  const SIGNUP_FORM = /sign-?up|register|registration|join|create/i;
  const qsa = (root, sel) =>
    (root.nodeType === 9 ? Document.prototype : Element.prototype).querySelectorAll.call(root, sel);
  const attr = (el, name) => Element.prototype.getAttribute.call(el, name) || '';
  const isInput = (el) => !!el && el.nodeType === 1 && el.tagName === 'INPUT';
  // A field shown with "show password" keeps counting as a password field.
  const isPassword = (el) => isInput(el) && (el.type === 'password' || (el.__moriPw === true && el.type === 'text'));
  const isTextLike = (el) => isInput(el) && ['text', 'email', 'tel'].includes(el.type) && !isPassword(el);
  const shown = (el) => {
    if (!el.isConnected || el.disabled) return false;
    const r = el.getBoundingClientRect();
    if (r.width < 4 || r.height < 4) return false;
    const s = getComputedStyle(el);
    return s.visibility !== 'hidden' && s.display !== 'none' && Number(s.opacity) > 0.05;
  };
  const auto = (el) => attr(el, 'autocomplete').toLowerCase();
  const hints = (el) => ['name', 'id', 'placeholder', 'aria-label'].map((a) => attr(el, a)).join(' ');
  const before = (a, b) => !!(a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING);
  const inputsIn = (root) => Array.from(qsa(root, 'input'));
  const scopeOf = (el) => {
    if (el.form) return el.form;
    let node = el.parentElement;
    for (let i = 0; node && node !== document.body && i < 6; i++, node = node.parentElement) {
      if (inputsIn(node).some((x) => x !== el && shown(x) && (isTextLike(x) || isPassword(x)))) return node;
    }
    return document.body || document.documentElement;
  };
  const classify = (scope) => {
    const pws = inputsIn(scope).filter((x) => isPassword(x) && shown(x));
    pws.forEach((x) => { x.__moriPw = true; });
    let current = [], fresh = [];
    const marked = pws.filter((x) => auto(x).includes('new-password'));
    if (marked.length) {
      fresh = marked;
      current = pws.filter((x) => !marked.includes(x));
    } else if (pws.length >= 3) {
      current = [pws[0]];
      fresh = pws.slice(1);
    } else if (pws.length === 2) {
      if (auto(pws[0]).includes('current-password') || CURRENTISH.test(hints(pws[0]))) {
        current = [pws[0]];
        fresh = [pws[1]];
      } else {
        fresh = pws;
      }
    } else if (pws.length === 1) {
      const p = pws[0];
      const formHint = scope.tagName === 'FORM'
        ? ['id', 'name', 'action', 'class'].map((a) => attr(scope, a)).join(' ')
        : '';
      const signup = NEWISH.test(hints(p)) || SIGNUP_FORM.test(formHint);
      if (!auto(p).includes('current-password') && signup) fresh = pws;
      else current = pws;
    }
    return { current, fresh };
  };
  const userScore = (x) => {
    const a = auto(x);
    if (a.includes('username')) return 5;
    if (a.includes('email') || x.type === 'email') return 4;
    if (USERISH.test(hints(x))) return 3;
    if (x.type === 'tel') return 2;
    return 1;
  };
  // The username field for `pw`: the most username-like field before it,
  // the closest on a tie; else a hidden one carrying the username (sites
  // whose password step only shows the account).
  const usernameFor = (pw, scope) => {
    let best = null, bestScore = 0;
    for (const x of inputsIn(scope)) {
      if (!before(x, pw)) continue;
      let score = 0;
      if (isTextLike(x) && shown(x)) score = userScore(x) + 1;
      else if ((isTextLike(x) || x.type === 'hidden') && auto(x).includes('username') && x.value) score = 1;
      if (score > 0 && score >= bestScore) {
        best = x;
        bestScore = score;
      }
    }
    return best;
  };
  // The only, clearly username-like field of a scope without a password
  // field: the first step of a two-step sign-in.
  const loneUsername = (scope) => {
    const fields = inputsIn(scope).filter((x) => isTextLike(x) && shown(x));
    return fields.length === 1 && userScore(fields[0]) >= 3 ? fields[0] : null;
  };
  const targetOf = (e) => (e.composedPath ? e.composedPath()[0] : e.target);
  const deepActive = () => {
    let a = document.activeElement;
    while (a && a.shadowRoot && a.shadowRoot.activeElement) a = a.shadowRoot.activeElement;
    return a;
  };
"#
    };
}

/// Watches the main frame for login forms and reports through the binding
/// (see [`super::report::FormReport`]): which forms the page shows, focus
/// on login fields (with their position, for the suggestions dropdown),
/// and submitted logins. A submission counts only for a real user action
/// (a click, Enter, or a form submit the user started); after one, the
/// agent tells whether the form went away. Install with
/// `Page.addScriptToEvaluateOnNewDocument` in the agent world. Idempotent.
pub const FORM_AGENT_SCRIPT: &str = concat!(
    "(() => {\n  if (window.top !== window || window.__moriPasswordAgent) return;\n  window.__moriPasswordAgent = true;",
    form_helpers_js!(),
    r#"
  const send = (msg) => {
    try {
      msg.origin = location.origin;
      globalThis.__moriPasswords(JSON.stringify(msg));
    } catch (e) {}
  };

  let reported = null;
  const scan = () => {
    let login = false, fresh = false;
    const seen = new Set();
    for (const pw of inputsIn(document)) {
      if (!isPassword(pw) || !shown(pw)) continue;
      const scope = scopeOf(pw);
      if (seen.has(scope)) continue;
      seen.add(scope);
      const c = classify(scope);
      login = login || c.current.length > 0;
      fresh = fresh || c.fresh.length > 0;
    }
    const key = login + '/' + fresh;
    if (key !== reported) {
      reported = key;
      send({ t: 'forms', login, newPassword: fresh });
    }
  };
  let scanTimer = 0;
  const scanSoon = () => {
    if (!scanTimer) scanTimer = setTimeout(() => { scanTimer = 0; scan(); }, 500);
  };
  new MutationObserver(scanSoon).observe(document, {
    subtree: true, childList: true, attributes: true, attributeFilter: ['type', 'hidden', 'disabled'],
  });
  document.addEventListener('DOMContentLoaded', scan);
  window.addEventListener('load', scan);
  scan();

  // Focus on login fields, for the suggestions dropdown.
  let focused = null, focusedAt = 0, typed = false, lastUserInput = 0;
  const kindOf = (el) => {
    if (!isInput(el) || !shown(el)) return null;
    const scope = scopeOf(el);
    const c = classify(scope);
    if (isPassword(el)) return c.fresh.includes(el) ? 'new-password' : 'password';
    if (!isTextLike(el)) return null;
    if (c.current.length) return usernameFor(c.current[0], scope) === el ? 'username' : null;
    if (c.fresh.length) return null;
    return loneUsername(scope) === el ? 'username' : null;
  };
  const rectOf = (el) => {
    const r = el.getBoundingClientRect();
    return { x: r.left, y: r.top, w: r.width, h: r.height };
  };
  const reportFocus = (el, kind, user) => {
    focused = el;
    focusedAt = Date.now();
    typed = false;
    send({ t: 'focus', field: kind, user, rect: rectOf(el), maxLength: el.maxLength > 0 ? el.maxLength : null });
  };
  document.addEventListener('focusin', (e) => {
    const el = targetOf(e);
    const kind = kindOf(el);
    scanSoon();
    if (kind) {
      reportFocus(el, kind, Date.now() - lastUserInput < 1000);
    } else if (focused) {
      focused = null;
      send({ t: 'blur' });
    }
  }, true);
  document.addEventListener('focusout', () => {
    setTimeout(() => {
      // Focus that left the page went to Mori (its dropdown): keep it.
      if (!focused || !document.hasFocus()) return;
      if (deepActive() !== focused) {
        focused = null;
        send({ t: 'blur' });
      }
    }, 0);
  }, true);
  document.addEventListener('input', (e) => {
    if (!e.isTrusted || typed || !focused || targetOf(e) !== focused) return;
    typed = true;
    send({ t: 'typing' });
  }, true);
  let moveQueued = false;
  const move = () => {
    if (!focused || moveQueued) return;
    moveQueued = true;
    requestAnimationFrame(() => {
      moveQueued = false;
      if (focused && focused.isConnected) send({ t: 'move', rect: rectOf(focused) });
    });
  };
  window.addEventListener('scroll', move, { capture: true, passive: true });
  window.addEventListener('resize', move, { passive: true });

  // Submitted logins.
  let lastKey = '', lastAt = 0;
  const watch = (fields) => {
    let ticks = 0;
    const timer = setInterval(() => {
      ticks += 1;
      if (!fields.some((f) => f.isConnected && shown(f))) {
        clearInterval(timer);
        send({ t: 'submitted' });
      } else if (ticks >= 20) {
        clearInterval(timer);
      }
    }, 500);
  };
  const capture = (scope) => {
    const c = classify(scope);
    const fields = c.current.concat(c.fresh);
    if (!fields.length) {
      const u = loneUsername(scope);
      if (u && u.value.trim()) send({ t: 'username', username: u.value });
      return;
    }
    if (c.fresh.length >= 2 && c.fresh[0].value !== c.fresh[1].value) return;
    const password = (c.fresh[0] && c.fresh[0].value) || (c.current[0] && c.current[0].value) || '';
    if (!password) return;
    const u = usernameFor(c.current[0] || c.fresh[0], scope);
    const username = u ? u.value : '';
    const key = username + '\n' + password;
    if (key === lastKey && Date.now() - lastAt < 3000) return;
    lastKey = key;
    lastAt = Date.now();
    setTimeout(() => { lastKey = ''; }, 3000);
    send({ t: 'submit', username, password });
    watch(fields);
  };
  document.addEventListener('submit', (e) => {
    if (e.isTrusted && e.target && e.target.tagName === 'FORM') capture(e.target);
  }, true);
  document.addEventListener('pointerdown', (e) => {
    if (e.isTrusted) lastUserInput = Date.now();
  }, true);
  document.addEventListener('keydown', (e) => {
    if (!e.isTrusted) return;
    lastUserInput = Date.now();
    const el = targetOf(e);
    if (e.key === 'Enter' && isInput(el)) capture(el.form || scopeOf(el));
  }, true);
  document.addEventListener('click', (e) => {
    if (!e.isTrusted) return;
    const el = targetOf(e);
    // A click on the focused field brings the dropdown back (the click
    // that focused it was reported by focusin).
    if (el && el === focused) {
      const kind = Date.now() - focusedAt > 300 ? kindOf(el) : null;
      if (kind) reportFocus(el, kind, true);
      return;
    }
    const button = el && el.closest
      ? el.closest('button,input[type=submit],input[type=image],input[type=button],[role=button]')
      : null;
    if (button) capture(button.form || scopeOf(button));
  }, true);
})()"#
);

/// What a fill script puts into the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FillMode {
    /// A saved login: the password (and the username, if given) into the
    /// focused login form, or the first one on the page.
    Login,
    /// A generated password into every new-password field of the focused
    /// sign-up or change-password form.
    NewPassword,
}

/// A script filling `password` (and `username`) into the page, only if the
/// main frame's origin is still `origin`. It sets each field's value and
/// fires `input` and `change`, as typing would, and evaluates to
/// `"filled"`, `"origin"` (the page changed) or `"no-form"`. Run it in the
/// agent world; the returned text holds the password.
pub fn fill_script(mode: FillMode, origin: &str, username: Option<&str>, password: &Secret) -> Secret {
    let js = |v: &str| serde_json::to_string(v).unwrap_or_else(|_| "\"\"".into());
    let username = username.map(js).unwrap_or_else(|| "null".into());
    let mode = match mode {
        FillMode::Login => "login",
        FillMode::NewPassword => "new",
    };
    Secret::new(format!(
        r#"(() => {{
  const origin = {origin};
  const username = {username};
  const password = {password};
  const mode = "{mode}";
  if (window.top !== window || location.origin !== origin) return 'origin';
{helpers}
  const setValue = (el, value) => {{
    if (el.readOnly) return;
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(el, value);
    el.dispatchEvent(new Event('input', {{ bubbles: true, composed: true }}));
    el.dispatchEvent(new Event('change', {{ bubbles: true }}));
  }};
  const active = deepActive();
  const scopes = [];
  if (isInput(active)) scopes.push(scopeOf(active));
  for (const pw of inputsIn(document)) if (isPassword(pw) && shown(pw)) scopes.push(scopeOf(pw));
  for (const scope of scopes) {{
    const c = classify(scope);
    if (mode === 'new' && c.fresh.length) {{
      c.fresh.forEach((f) => setValue(f, password));
      return 'filled';
    }}
    if (mode === 'login' && c.current.length) {{
      setValue(c.current[0], password);
      const u = username === null ? null : usernameFor(c.current[0], scope);
      if (u && u.type !== 'hidden') setValue(u, username);
      return 'filled';
    }}
  }}
  if (mode === 'login' && username !== null && isInput(active)) {{
    const u = loneUsername(scopeOf(active));
    if (u) {{
      setValue(u, username);
      return 'filled';
    }}
  }}
  return 'no-form';
}})()"#,
        origin = js(origin),
        password = js(password.expose()),
        helpers = form_helpers_js!(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::passwords::report::AGENT_BINDING;

    #[test]
    fn agent_reports_through_the_binding() {
        assert!(FORM_AGENT_SCRIPT.starts_with("(() => {"));
        assert!(FORM_AGENT_SCRIPT.ends_with("})()"));
        assert!(FORM_AGENT_SCRIPT.contains(&format!("globalThis.{AGENT_BINDING}(")));
        assert!(FORM_AGENT_SCRIPT.contains("window.top !== window"), "main frame only");
        assert!(FORM_AGENT_SCRIPT.contains("e.isTrusted"), "user actions only");
    }

    #[test]
    fn fill_scripts_escape_and_check_origin() {
        let pw = Secret::new("a\"b</script>\\".into());
        let s = fill_script(FillMode::Login, "https://a.test", Some("me\n"), &pw);
        let s = s.expose();
        assert!(s.contains(r#"const origin = "https://a.test";"#));
        assert!(s.contains(r#"const username = "me\n";"#));
        assert!(s.contains(r#"const password = "a\"b</script>\\";"#));
        assert!(s.contains("location.origin !== origin"));
        assert!(s.starts_with("(() => {") && s.ends_with("})()"));
        let s = fill_script(FillMode::NewPassword, "https://a.test", None, &pw);
        assert!(s.expose().contains("const username = null;"));
        assert!(s.expose().contains(r#"const mode = "new";"#));
    }
}
