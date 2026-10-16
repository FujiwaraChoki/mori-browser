//! Page scripts behind the assistant's browser tools (ported from the macOS
//! `BrowserAutomation.swift`).
//!
//! Each script is an expression evaluated through DevTools `Runtime.evaluate`
//! (`awaitPromise`, `returnByValue`). Arguments are spliced in as one JSON
//! object literal, so no caller-supplied text is ever interpolated into code.
//! Scripts catch their own exceptions and report `{ error }` instead, which
//! the tool layer turns into a failed result.
//!
//! Elements reported by [`read_page`] carry a stable `ref` ("m12") kept in an
//! off-DOM `WeakRef` registry (`window.__moriRefs`), so later actions can
//! target the exact element without mutating the page.

use serde_json::{Value, json};

/// Shared helpers prepended to every action script.
const PRELUDE: &str = r##"
  const clean = (value) => String(value || "").replace(/\s+/g, " ").trim();
  const isSensitive = (el) => {
    if (!el || el.nodeType !== 1) return false;
    if (el.localName === "input" && String(el.type || "").toLowerCase() === "password") return true;
    const ac = String(el.autocomplete || el.getAttribute("autocomplete") || "").toLowerCase();
    if (/(current-password|new-password|one-time-code|cc-number|cc-csc)/.test(ac)) return true;
    const hint = (String(el.name || "") + " " + String(el.id || "")).toLowerCase();
    return /(password|passwd|\botp\b|\btotp\b|\bcvv\b|\bcvc\b|cardnumber)/.test(hint);
  };
  const byRef = (key) => {
    const registry = window.__moriRefs;
    if (!registry || !registry.map.has(key)) return null;
    const el = registry.map.get(key).deref();
    return (el && el.isConnected) ? el : null;
  };
  const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
"##;

fn wrap(body: &str, args: Value) -> String {
    format!(
        "(async (args) => {{\n  try {{\n{PRELUDE}\n{body}\n  }} catch (e) {{\n    return {{ error: String((e && e.message) || e) }};\n  }}\n}})({args})"
    )
}

/// Title, URL, visible text (paged by `text_offset`/`max_chars`), selected
/// text, links and form controls with refs, viewport and scroll position.
/// Values of password/OTP/card fields are never included.
pub fn read_page(max_chars: usize, text_offset: usize) -> String {
    wrap(
        r##"
  const max = Math.max(500, args.max);
  const textOffset = Math.max(0, args.offset);
  const registry = window.__moriRefs || (window.__moriRefs = { seq: 0, map: new Map() });
  const refFor = (el) => {
    for (const [key, held] of registry.map) {
      if (held.deref() === el) return key;
    }
    if (registry.map.size > 600) {
      for (const [key, held] of registry.map) {
        if (!held.deref()) registry.map.delete(key);
      }
    }
    const key = "m" + (++registry.seq);
    registry.map.set(key, new WeakRef(el));
    return key;
  };
  const pathFor = (el) => {
    if (!el || el.nodeType !== 1) return "";
    if (el.id) return "#" + CSS.escape(el.id);
    const parts = [];
    let node = el;
    while (node && node.nodeType === 1 && parts.length < 5) {
      let part = node.localName || "element";
      if (node.classList && node.classList.length) {
        part += "." + Array.from(node.classList).slice(0, 2).map(CSS.escape).join(".");
      }
      const parent = node.parentElement;
      if (parent) {
        const siblings = Array.from(parent.children).filter((child) => child.localName === node.localName);
        if (siblings.length > 1) part += `:nth-of-type(${siblings.indexOf(node) + 1})`;
      }
      parts.unshift(part);
      node = parent;
    }
    return parts.join(" > ");
  };
  const isVisible = (el) => {
    const rect = el.getBoundingClientRect();
    const style = getComputedStyle(el);
    return rect.width > 0 && rect.height > 0 && style.visibility !== "hidden" && style.display !== "none";
  };
  const links = Array.from(document.links).filter(isVisible).slice(0, 80).map((el) => ({
    text: clean(el.innerText || el.textContent).slice(0, 160),
    href: el.href,
    ref: refFor(el),
  }));
  const controls = Array.from(document.querySelectorAll("button,input,textarea,select,[role=button],[contenteditable=true]"))
    .filter(isVisible)
    .slice(0, 120)
    .map((el) => {
      const sensitive = isSensitive(el);
      const r = el.getBoundingClientRect();
      return {
        tag: el.localName,
        role: el.getAttribute("role") || "",
        type: el.getAttribute("type") || "",
        name: el.getAttribute("name") || "",
        text: sensitive
          ? clean(el.getAttribute("aria-label") || el.getAttribute("placeholder") || "").slice(0, 160)
          : clean(el.innerText || el.value || el.getAttribute("aria-label") || el.getAttribute("placeholder")).slice(0, 160),
        sensitive,
        ref: refFor(el),
        selector: pathFor(el),
        rect: { x: Math.round(r.x), y: Math.round(r.y), width: Math.round(r.width), height: Math.round(r.height) }
      };
    });
  const fullText = clean(document.body ? document.body.innerText : "");
  const offset = Math.min(textOffset, fullText.length);
  const visibleText = fullText.slice(offset, offset + max);
  return {
    title: document.title,
    url: location.href,
    selectedText: String(getSelection ? getSelection() : "").slice(0, 2000),
    visibleText,
    textRange: { offset, returned: visibleText.length, total: fullText.length },
    links,
    controls,
    viewport: { width: innerWidth, height: innerHeight, devicePixelRatio },
    scroll: { x: scrollX, y: scrollY, maxY: Math.max(0, document.documentElement.scrollHeight - innerHeight) }
  };"##,
        json!({ "max": max_chars, "offset": text_offset }),
    )
}

/// Arguments of [`page_action`]: an element target plus action parameters.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
pub struct ActionArgs {
    pub action: String,
    pub r#ref: String,
    pub selector: String,
    pub text: String,
    pub key: String,
    pub direction: String,
    pub x: f64,
    pub y: f64,
    pub amount: f64,
    pub duration: u64,
}

/// click, doubleClick, hover, hold, type, keyPress or scroll on the element
/// named by ref / selector / coordinates (else the focused element). Typing
/// into credential fields is refused.
pub fn page_action(args: &ActionArgs) -> String {
    wrap(
        r##"
  const { action, ref, selector, text, key, direction, x, y, amount, duration } = args;
  const intoView = (el) => {
    try { el.scrollIntoView({ block: "center", inline: "center" }); } catch (e) {}
  };
  const refOf = (el) => {
    const registry = window.__moriRefs;
    if (!registry || !el) return "";
    for (const [k, held] of registry.map) {
      if (held.deref() === el) return k;
    }
    return "";
  };
  let staleRef = false;
  const target = () => {
    if (ref) {
      const el = byRef(ref);
      if (el) { intoView(el); return el; }
      staleRef = true;
      return null;
    }
    if (selector) {
      const el = document.querySelector(selector);
      if (el) intoView(el);
      return el;
    }
    if (x >= 0 && y >= 0) return document.elementFromPoint(x, y);
    return document.activeElement || document.body;
  };
  const describe = (el) => {
    if (!el) return { found: false };
    const r = el.getBoundingClientRect();
    const label = isSensitive(el)
      ? clean(el.getAttribute("aria-label") || el.getAttribute("placeholder") || "").slice(0, 160)
      : clean(el.innerText || el.value || el.getAttribute("aria-label") || el.getAttribute("title") || "").slice(0, 160);
    return {
      found: true,
      tag: el.localName,
      id: el.id || "",
      ref: refOf(el),
      text: label,
      rect: { x: Math.round(r.x), y: Math.round(r.y), width: Math.round(r.width), height: Math.round(r.height) }
    };
  };
  const pointFor = (el) => {
    const r = el.getBoundingClientRect();
    return { cx: x >= 0 ? x : r.left + r.width / 2, cy: y >= 0 ? y : r.top + r.height / 2 };
  };
  const mouse = (el, type, detail = 1) => {
    const p = pointFor(el);
    el.dispatchEvent(new MouseEvent(type, { bubbles: true, cancelable: true, view: window, clientX: p.cx, clientY: p.cy, detail }));
  };
  const pointer = (el, type) => {
    const p = pointFor(el);
    try {
      el.dispatchEvent(new PointerEvent(type, { bubbles: true, cancelable: true, view: window, clientX: p.cx, clientY: p.cy, pointerId: 1, pointerType: "mouse", isPrimary: true }));
    } catch (e) {}
  };
  const el = target();
  if (staleRef) {
    return { error: "Ref " + ref + " no longer exists — the page has changed since the last read. Re-read the page to get fresh refs." };
  }
  if (["click", "doubleClick", "hover", "hold", "type"].includes(action) && !el) {
    return { error: "No target element matched the selector or coordinates. Re-read the page and target an element by its ref." };
  }
  if (action === "click" || action === "doubleClick") {
    pointer(el, "pointerover");
    mouse(el, "mousemove");
    pointer(el, "pointerdown");
    mouse(el, "mousedown");
    if (typeof el.focus === "function") el.focus();
    pointer(el, "pointerup");
    mouse(el, "mouseup");
    mouse(el, "click");
    if (action === "doubleClick") mouse(el, "dblclick", 2);
    return { action, target: describe(el), url: location.href };
  }
  if (action === "hover") {
    pointer(el, "pointerover");
    mouse(el, "mousemove");
    mouse(el, "mouseover");
    return { action, target: describe(el) };
  }
  if (action === "hold") {
    pointer(el, "pointerdown");
    mouse(el, "mousedown");
    await sleep(duration);
    pointer(el, "pointerup");
    mouse(el, "mouseup");
    return { action, durationMS: duration, target: describe(el) };
  }
  if (action === "type") {
    if (isSensitive(el)) {
      return { error: "Refused: this looks like a password/2FA/card field. Mori never types secrets — click the field and let the user's password manager autofill it, or use the site's passkey sign-in." };
    }
    if (typeof el.focus === "function") el.focus();
    if (el.localName === "select") {
      const options = Array.from(el.options);
      const wanted = text.trim().toLowerCase();
      const option = options.find((o) => o.value === text)
        || options.find((o) => clean(o.label).toLowerCase() === wanted || clean(o.text).toLowerCase() === wanted);
      if (!option) {
        return { error: "No option matched " + JSON.stringify(text) + ". Available: " + options.slice(0, 30).map((o) => o.value || clean(o.text)).join(", ") };
      }
      el.value = option.value;
      el.dispatchEvent(new Event("input", { bubbles: true }));
      el.dispatchEvent(new Event("change", { bubbles: true }));
      return { action, selected: option.value, target: describe(el) };
    }
    if (el.isContentEditable) {
      document.execCommand("selectAll", false, null);
      document.execCommand("insertText", false, text);
      return { action, target: describe(el) };
    }
    if ("value" in el) {
      const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
      const valueSetter = (Object.getOwnPropertyDescriptor(proto, "value") || {}).set;
      if (valueSetter) { valueSetter.call(el, text); } else { el.value = text; }
      el.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: text }));
      el.dispatchEvent(new Event("change", { bubbles: true }));
      return { action, target: describe(el) };
    }
    document.execCommand("insertText", false, text);
    return { action, target: describe(el) };
  }
  if (action === "keyPress") {
    const active = el || document.activeElement || document.body;
    if (key.length === 1 && isSensitive(active)) {
      return { error: "Refused: character key presses into a password/2FA/card field are not allowed. Let the user's password manager fill it; Enter is allowed to submit." };
    }
    if (typeof active.focus === "function") active.focus();
    const opts = { key, bubbles: true, cancelable: true };
    const proceed = active.dispatchEvent(new KeyboardEvent("keydown", opts));
    active.dispatchEvent(new KeyboardEvent("keypress", opts));
    active.dispatchEvent(new KeyboardEvent("keyup", opts));
    if (proceed && key === "Enter") {
      const form = active.form || (active.closest ? active.closest("form") : null);
      if (form) {
        if (typeof form.requestSubmit === "function") { form.requestSubmit(); } else { form.submit(); }
        return { action, key, submittedForm: true, target: describe(active) };
      }
    }
    return { action, key, target: describe(active) };
  }
  if (action === "scroll") {
    const dx = direction === "left" ? -amount : (direction === "right" ? amount : 0);
    const dy = direction === "up" ? -amount : (direction === "down" ? amount : 0);
    const scrollTargeted = (ref || selector) && el && el !== document.body;
    if (scrollTargeted) {
      el.scrollBy({ left: dx, top: dy, behavior: "auto" });
    } else {
      window.scrollBy({ left: dx, top: dy, behavior: "auto" });
    }
    await sleep(60);
    return {
      action,
      scroll: { x: scrollX, y: scrollY, maxY: Math.max(0, document.documentElement.scrollHeight - innerHeight) },
      target: scrollTargeted ? describe(el) : null
    };
  }
  return { error: "Unsupported page action: " + action };"##,
        serde_json::to_value(args).unwrap_or_default(),
    )
}

/// One field of a `fillForm` call.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
pub struct FormField {
    pub r#ref: String,
    pub selector: String,
    pub text: String,
}

/// Fill several fields at once; returns `{filled, total, outcomes}`.
/// Credential fields are skipped.
pub fn fill_form(fields: &[FormField]) -> String {
    wrap(
        r##"
  const find = (f) => {
    if (f.ref) return byRef(f.ref);
    if (f.selector) { try { return document.querySelector(f.selector); } catch (e) { return null; } }
    return null;
  };
  const label = (f) => f.ref || f.selector || "(no target)";
  const outcomes = args.fields.map((f) => {
    const el = find(f);
    if (!el) return { field: label(f), ok: false, reason: "no matching element (stale ref? re-read the page)" };
    if (isSensitive(el)) return { field: label(f), ok: false, reason: "credential field — left for the user's password manager / passkey flow" };
    try { el.scrollIntoView({ block: "center", inline: "center" }); } catch (e) {}
    if (typeof el.focus === "function") el.focus();
    if (el.localName === "select") {
      const options = Array.from(el.options);
      const wanted = f.text.trim().toLowerCase();
      const option = options.find((o) => o.value === f.text)
        || options.find((o) => clean(o.label).toLowerCase() === wanted || clean(o.text).toLowerCase() === wanted);
      if (!option) return { field: label(f), ok: false, reason: "no matching option" };
      el.value = option.value;
      el.dispatchEvent(new Event("input", { bubbles: true }));
      el.dispatchEvent(new Event("change", { bubbles: true }));
      return { field: label(f), ok: true, selected: option.value };
    }
    if (el.isContentEditable) {
      document.execCommand("selectAll", false, null);
      document.execCommand("insertText", false, f.text);
      return { field: label(f), ok: true };
    }
    if ("value" in el) {
      const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
      const valueSetter = (Object.getOwnPropertyDescriptor(proto, "value") || {}).set;
      try {
        if (valueSetter) { valueSetter.call(el, f.text); } else { el.value = f.text; }
      } catch (e) {
        return { field: label(f), ok: false, reason: String((e && e.message) || e) };
      }
      el.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: f.text }));
      el.dispatchEvent(new Event("change", { bubbles: true }));
      return { field: label(f), ok: true };
    }
    return { field: label(f), ok: false, reason: "element is not editable" };
  });
  return { action: "fillForm", filled: outcomes.filter((o) => o.ok).length, total: args.fields.length, outcomes };"##,
        json!({ "fields": fields }),
    )
}

/// Poll until a ref/selector matches a visible element or `text` appears.
pub fn wait_for(r#ref: &str, selector: &str, text: &str, timeout_ms: u64) -> String {
    wrap(
        r##"
  const { ref, selector, text, timeout } = args;
  if (!ref && !selector && !text) {
    return { error: "waitFor needs a ref, selector, or text to wait for." };
  }
  const deadline = Date.now() + timeout;
  const check = () => {
    if (text && (document.body ? document.body.innerText : "").includes(text)) return { matched: "text" };
    let el = null;
    if (ref) el = byRef(ref);
    else if (selector) { try { el = document.querySelector(selector); } catch (e) { return { error: "Invalid selector." }; } }
    if (!el) return null;
    const r = el.getBoundingClientRect();
    return (r.width > 0 && r.height > 0) ? { matched: ref ? "ref" : "selector" } : null;
  };
  while (Date.now() < deadline) {
    const hit = check();
    if (hit && hit.error) return hit;
    if (hit) return { found: true, matched: hit.matched, waitedMS: timeout - (deadline - Date.now()) };
    await sleep(150);
  }
  return { found: false, timedOutMS: timeout, hint: "The expected content did not appear. Re-read the page to see its current state." };"##,
        json!({ "ref": r#ref, "selector": selector, "text": text, "timeout": timeout_ms }),
    )
}

/// Count case-insensitive matches of `text` in the page, return a few
/// snippets of context, and select/scroll to the first match.
pub fn find_text(text: &str) -> String {
    wrap(
        r##"
  const needle = String(args.text || "");
  if (!needle.trim()) return { error: "findText needs non-empty text." };
  const body = clean(document.body ? document.body.innerText : "");
  const hay = body.toLowerCase();
  const lower = needle.toLowerCase();
  let count = 0;
  const snippets = [];
  let at = hay.indexOf(lower);
  while (at !== -1 && count < 1000) {
    count++;
    if (snippets.length < 5) {
      snippets.push(body.slice(Math.max(0, at - 80), Math.min(body.length, at + needle.length + 80)));
    }
    at = hay.indexOf(lower, at + Math.max(1, lower.length));
  }
  let highlighted = false;
  try {
    const sel = getSelection();
    if (sel) sel.removeAllRanges();
    highlighted = !!(window.find && window.find(needle, false, false, true));
  } catch (e) {}
  return { text: needle, matches: count, snippets, highlighted };"##,
        json!({ "text": text }),
    )
}

/// Whether any of the given ref/selector targets is a credential field, so
/// the approval card hides the text about to be typed. Only a boolean leaves
/// the page, and only to Mori's UI.
pub fn targets_sensitive(targets: &[FormField]) -> String {
    wrap(
        r##"
  const find = (t) => {
    if (t.ref) return byRef(t.ref);
    if (t.selector) { try { return document.querySelector(t.selector); } catch (e) { return null; } }
    return null;
  };
  return args.targets.some((t) => isSensitive(find(t)));"##,
        json!({ "targets": targets }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_are_single_expressions_with_inlined_args() {
        let typed = page_action(&ActionArgs {
            action: "type".into(),
            text: "\"); alert(1); (\"".into(),
            ..Default::default()
        });
        assert!(typed.starts_with("(async (args) => {"));
        // Caller text only ever appears inside the trailing JSON argument.
        let (body, call_args) = typed.rsplit_once("})(").expect("invocation");
        assert!(!body.contains("alert(1)"));
        assert!(call_args.contains(r##""text":"\"); alert(1); (\"""##));
        for s in [
            read_page(8000, 0),
            fill_form(&[FormField {
                text: "a".into(),
                ..Default::default()
            }]),
            wait_for("m1", "", "", 500),
            find_text("x"),
            targets_sensitive(&[]),
        ] {
            assert!(s.starts_with("(async (args) => {"), "{s}");
            assert!(s.contains("catch (e)"));
            assert!(s.trim_end().ends_with(')'));
        }
        assert!(read_page(100, 5).ends_with(r##"({"max":100,"offset":5})"##));
    }
}
