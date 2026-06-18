import SwiftUI

/// A per-site "Boost": custom CSS and JS injected into matching pages, plus a
/// list of zapped element selectors (Arc-style click-to-remove). Matched by host
/// suffix, so a boost saved for `youtube.com` also covers `www.youtube.com`.
struct SiteBoost: Codable, Identifiable {
    var id = UUID()
    var host: String
    var css: String = ""
    var js: String = ""
    var zappedSelectors: [String] = []
    var enabled: Bool = true
    var updatedAt: Date = Date()

    var isEmpty: Bool {
        css.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && js.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && zappedSelectors.isEmpty
    }
}

/// Persistent store of per-site Boosts, JSON-backed in Application Support.
final class BoostStore: ObservableObject {
    static let shared = BoostStore()

    @Published private(set) var boosts: [SiteBoost] = []

    private let fileURL: URL
    private var saveScheduled = false

    init() {
        let fm = FileManager.default
        let base = fm.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
            ?? fm.temporaryDirectory
        let dir = base.appendingPathComponent("MoriBrowser", isDirectory: true)
        try? fm.createDirectory(at: dir, withIntermediateDirectories: true)
        fileURL = dir.appendingPathComponent("boosts.json")
        load()
    }

    // MARK: Host helpers

    /// The host a Boost is keyed on for a given URL: lowercased, `www.` stripped.
    static func normalizedHost(forURL url: String) -> String? {
        guard let host = URLComponents(string: url)?.host?.lowercased(), !host.isEmpty
        else { return nil }
        return host.hasPrefix("www.") ? String(host.dropFirst(4)) : host
    }

    /// The Boost whose host matches this URL (exact or subdomain), if any.
    func boost(forURL url: String) -> SiteBoost? {
        guard let host = URLComponents(string: url)?.host?.lowercased() else { return nil }
        return boosts.first { boost in
            host == boost.host || host.hasSuffix("." + boost.host)
        }
    }

    func boost(forHost host: String) -> SiteBoost? {
        boosts.first { $0.host == host }
    }

    /// Fetch the existing Boost for this host, or a blank one ready to edit.
    func editableBoost(forHost host: String) -> SiteBoost {
        boost(forHost: host) ?? SiteBoost(host: host)
    }

    // MARK: Mutation

    func upsert(_ boost: SiteBoost) {
        var copy = boost
        copy.updatedAt = Date()
        if let idx = boosts.firstIndex(where: { $0.id == boost.id }) {
            boosts[idx] = copy
        } else if let idx = boosts.firstIndex(where: { $0.host == boost.host }) {
            boosts[idx] = copy
        } else {
            boosts.insert(copy, at: 0)
        }
        // Drop a Boost that's been emptied out so the list doesn't accumulate
        // no-op entries.
        boosts.removeAll { $0.isEmpty }
        scheduleSave()
    }

    func addZaps(_ selectors: [String], host: String) {
        guard !selectors.isEmpty else { return }
        var boost = editableBoost(forHost: host)
        for selector in selectors where !boost.zappedSelectors.contains(selector) {
            boost.zappedSelectors.append(selector)
        }
        upsert(boost)
    }

    func clearZaps(host: String) {
        guard var boost = boost(forHost: host) else { return }
        boost.zappedSelectors.removeAll()
        upsert(boost)
    }

    func remove(host: String) {
        boosts.removeAll { $0.host == host }
        scheduleSave()
    }

    func setEnabled(_ enabled: Bool, host: String) {
        guard var boost = boost(forHost: host) else { return }
        boost.enabled = enabled
        upsert(boost)
    }

    // MARK: Injection

    /// The idempotent script that applies this URL's Boost (CSS + zaps + once-JS).
    /// Returns nil when there's nothing enabled to apply.
    func injectionScript(forURL url: String) -> String? {
        guard let boost = boost(forURL: url), boost.enabled, !boost.isEmpty else { return nil }
        let zapCSS = boost.zappedSelectors.isEmpty
            ? ""
            : boost.zappedSelectors.joined(separator: ",\n") + " { display: none !important; }\n"
        let css = zapCSS + boost.css
        let jsLiteralCSS = Self.jsString(css)
        let jsLiteralJS = Self.jsString(boost.js)
        return """
        (() => {
          const css = \(jsLiteralCSS);
          const js = \(jsLiteralJS);
          let style = document.getElementById('__mori_boost_style');
          if (!style) {
            style = document.createElement('style');
            style.id = '__mori_boost_style';
            (document.head || document.documentElement).appendChild(style);
          }
          style.textContent = css;
          if (js && !window.__moriBoostJSDone) {
            window.__moriBoostJSDone = true;
            try { (new Function(js))(); } catch (e) { console.error('Mori boost JS error', e); }
          }
        })();
        """
    }

    /// JSON-encode a string into a safe JS string literal (quotes included).
    static func jsString(_ value: String) -> String {
        guard let data = try? JSONSerialization.data(withJSONObject: [value], options: []),
              let array = String(data: data, encoding: .utf8),
              array.count >= 2
        else { return "\"\"" }
        return String(array.dropFirst().dropLast())
    }

    // MARK: Persistence

    private func load() {
        guard let data = try? Data(contentsOf: fileURL),
              let decoded = try? JSONDecoder().decode([SiteBoost].self, from: data)
        else { return }
        boosts = decoded
    }

    private func scheduleSave() {
        guard !saveScheduled else { return }
        saveScheduled = true
        DispatchQueue.main.async { [weak self] in
            self?.saveScheduled = false
            self?.save()
        }
    }

    private func save() {
        guard let data = try? JSONEncoder().encode(boosts) else { return }
        try? data.write(to: fileURL, options: .atomic)
    }
}

/// Reusable page scripts for the click-to-zap element picker.
enum BoostScripts {
    /// Injects a hover-highlight overlay and a capturing click handler that hides
    /// the clicked element and records a selector for it in `window.__moriZapped`.
    static let zapPicker = """
    (() => {
      if (window.__moriZapActive) return;
      window.__moriZapActive = true;
      window.__moriZapped = window.__moriZapped || [];
      const esc = (s) => (window.CSS && CSS.escape) ? CSS.escape(s) : s;
      const selectorFor = (el) => {
        if (!el || el.nodeType !== 1) return "";
        if (el.id) return "#" + esc(el.id);
        const parts = [];
        let node = el;
        while (node && node.nodeType === 1 && parts.length < 6 && node !== document.body) {
          let part = node.localName || "*";
          if (node.classList && node.classList.length) {
            part += "." + Array.from(node.classList).slice(0, 2).map(esc).join(".");
          }
          const parent = node.parentElement;
          if (parent) {
            const sibs = Array.from(parent.children).filter((c) => c.localName === node.localName);
            if (sibs.length > 1) part += `:nth-of-type(${sibs.indexOf(node) + 1})`;
          }
          parts.unshift(part);
          node = parent;
        }
        return parts.join(" > ");
      };
      const hl = document.createElement('div');
      hl.id = '__mori_zap_hl';
      hl.style.cssText = 'position:fixed;z-index:2147483647;pointer-events:none;background:rgba(255,64,64,0.22);border:2px solid rgba(255,40,40,0.95);border-radius:4px;box-shadow:0 0 0 9999px rgba(0,0,0,0.04);transition:left .04s,top .04s,width .04s,height .04s;';
      document.documentElement.appendChild(hl);
      const move = (e) => {
        const el = document.elementFromPoint(e.clientX, e.clientY);
        if (!el || el === hl) return;
        window.__moriZapTarget = el;
        const r = el.getBoundingClientRect();
        hl.style.left = r.left + 'px';
        hl.style.top = r.top + 'px';
        hl.style.width = r.width + 'px';
        hl.style.height = r.height + 'px';
      };
      const click = (e) => {
        e.preventDefault();
        e.stopPropagation();
        const el = window.__moriZapTarget;
        if (!el) return;
        const sel = selectorFor(el);
        if (sel) {
          window.__moriZapped.push(sel);
          el.style.setProperty('display', 'none', 'important');
        }
      };
      window.__moriZapMove = move;
      window.__moriZapClick = click;
      document.addEventListener('mousemove', move, true);
      document.addEventListener('click', click, true);
    })();
    """

    /// Tears the picker down and returns the collected selectors.
    static let collectZaps = """
    (() => {
      const zapped = (window.__moriZapped || []).slice();
      if (window.__moriZapMove) document.removeEventListener('mousemove', window.__moriZapMove, true);
      if (window.__moriZapClick) document.removeEventListener('click', window.__moriZapClick, true);
      const hl = document.getElementById('__mori_zap_hl');
      if (hl && hl.remove) hl.remove();
      window.__moriZapActive = false;
      window.__moriZapped = [];
      return zapped;
    })();
    """
}
