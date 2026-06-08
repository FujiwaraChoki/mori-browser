import Foundation

enum BrowserAutomationError: LocalizedError {
    case browserUnavailable
    case pageScriptFailed(String)
    case missingArgument(String)
    case unsupportedAction(String)
    case tabNotFound(String)

    var errorDescription: String? {
        switch self {
        case .browserUnavailable:
            return "The active browser view is not ready yet."
        case .pageScriptFailed(let message):
            return message
        case .missingArgument(let name):
            return "Missing required argument: \(name)."
        case .unsupportedAction(let action):
            return "Unsupported browser action: \(action)."
        case .tabNotFound(let id):
            return "No tab matched \(id)."
        }
    }
}

struct BrowserToolResult {
    let text: String
    let success: Bool

    var rpcResult: [String: Any] {
        [
            "contentItems": [
                ["type": "inputText", "text": text]
            ],
            "success": success
        ]
    }
}

enum BrowserAutomation {
    static let dynamicTools: [[String: Any]] = [
        [
            "name": "mori_browser_snapshot",
            "description": "Read Mori's open tabs and, by default, the active page. Returns tab IDs, titles, URLs, loading state, selected text, visible page text, links, form controls, viewport and scroll position.",
            "inputSchema": [
                "type": "object",
                "properties": [
                    "includePage": [
                        "type": "boolean",
                        "description": "Whether to read the active page in addition to tab metadata. Defaults to true."
                    ],
                    "maxTextChars": [
                        "type": "integer",
                        "description": "Maximum visible text characters to return from the page. Defaults to 8000."
                    ]
                ]
            ]
        ],
        [
            "name": "mori_browser_action",
            "description": "Perform browser and page actions in Mori. Supports openTab, selectTab, navigate, back, forward, reload, readPage, click, doubleClick, hover, hold, type, keyPress, scroll, findText and wait. Prefer selectors when available; use x/y viewport coordinates when selectors are not available.",
            "inputSchema": [
                "type": "object",
                "properties": [
                    "action": [
                        "type": "string",
                        "enum": [
                            "openTab", "selectTab", "navigate", "back", "forward",
                            "reload", "readPage", "click", "doubleClick", "hover",
                            "hold", "type", "keyPress", "scroll", "findText", "wait"
                        ]
                    ],
                    "tabId": ["type": "string"],
                    "url": ["type": "string"],
                    "selector": ["type": "string"],
                    "x": ["type": "number"],
                    "y": ["type": "number"],
                    "text": ["type": "string"],
                    "key": ["type": "string"],
                    "direction": [
                        "type": "string",
                        "enum": ["up", "down", "left", "right"]
                    ],
                    "amount": ["type": "number"],
                    "durationMS": ["type": "integer"],
                    "maxTextChars": ["type": "integer"]
                ],
                "required": ["action"]
            ]
        ],
        [
            "name": "mori_get_settings",
            "description": "Read Mori's current browser settings: homepage, new-tab behavior, search engine (and custom template), appearance theme, sidebar visibility and position, auto Picture-in-Picture, and the active gradient theme preset.",
            "inputSchema": [
                "type": "object",
                "properties": [:]
            ]
        ],
        [
            "name": "mori_update_settings",
            "description": "Change one or more of Mori's browser settings. Only the fields you provide are changed; omit a field to leave it untouched. Changes persist and apply live.",
            "inputSchema": [
                "type": "object",
                "properties": [
                    "homepageURL": [
                        "type": "string",
                        "description": "The page opened at launch and by 'new tab → homepage'. Accepts a full URL or 'mori://newtab/' for the built-in start page."
                    ],
                    "newTabBehavior": [
                        "type": "string",
                        "enum": ["homepage", "blank"],
                        "description": "What a freshly opened tab loads."
                    ],
                    "searchEngine": [
                        "type": "string",
                        "enum": ["google", "duckduckgo", "bing", "brave", "custom"],
                        "description": "Default search engine for address-bar queries. Use 'custom' together with customSearchTemplate."
                    ],
                    "customSearchTemplate": [
                        "type": "string",
                        "description": "Search URL used when searchEngine is 'custom'. Include '{query}' where the search terms go, e.g. 'https://example.com/search?q={query}'."
                    ],
                    "theme": [
                        "type": "string",
                        "enum": ["system", "light", "dark"],
                        "description": "Appearance theme. 'system' follows macOS."
                    ],
                    "showSidebarOnLaunch": [
                        "type": "boolean",
                        "description": "Whether the tab sidebar is shown when the window opens."
                    ],
                    "sidebarPosition": [
                        "type": "string",
                        "enum": ["left", "right"],
                        "description": "Which side of the window hosts the tab sidebar."
                    ],
                    "autoPiP": [
                        "type": "boolean",
                        "description": "Automatically enter Picture-in-Picture when switching away from a tab playing video."
                    ],
                    "gradientTheme": [
                        "type": "string",
                        "enum": [
                            "none", "evangelion", "tokyo-ghoul", "demon-slayer",
                            "jujutsu-kaisen", "chainsaw-man", "your-name", "sailor-moon"
                        ],
                        "description": "Apply a curated gradient chrome theme by preset id, or 'none' to clear the custom theme."
                    ]
                ]
            ]
        ]
    ]

    @MainActor
    static func handle(tool: String,
                       arguments: [String: Any],
                       store: BrowserStore) async -> BrowserToolResult {
        do {
            switch tool {
            case "mori_browser_snapshot":
                let text = try await snapshot(arguments: arguments, store: store)
                return BrowserToolResult(text: text, success: true)
            case "mori_browser_action":
                return try await action(arguments: arguments, store: store)
            case "mori_get_settings":
                let text = try getSettings(store: store)
                return BrowserToolResult(text: text, success: true)
            case "mori_update_settings":
                return try updateSettings(arguments: arguments, store: store)
            default:
                throw BrowserAutomationError.unsupportedAction(tool)
            }
        } catch {
            return BrowserToolResult(
                text: "Browser tool failed: \(error.localizedDescription)",
                success: false
            )
        }
    }

    @MainActor
    private static func snapshot(arguments: [String: Any],
                                 store: BrowserStore) async throws -> String {
        let includePage = bool(arguments["includePage"]) ?? true
        let maxTextChars = int(arguments["maxTextChars"]) ?? 8_000
        var payload: [String: Any] = [
            "selectedTabId": store.selectedTab?.id.uuidString ?? "",
            "tabs": store.tabs.map(tabRecord)
        ]

        if includePage, let tab = store.selectedTab {
            // Don't fail the whole snapshot if the active page isn't ready yet:
            // the tab list is useful on its own, and snapshot is often the very
            // first call (right after launch or openTab) before the browser has
            // finished creating. Surface the reason instead so the agent can wait
            // or act, rather than getting an opaque "not ready" failure.
            do {
                payload["activePage"] = try await readPage(tab: tab, maxTextChars: maxTextChars, store: store)
            } catch {
                payload["activePageError"] = error.localizedDescription
            }
        }

        return prettyJSON(payload)
    }

    @MainActor
    private static func action(arguments: [String: Any],
                               store: BrowserStore) async throws -> BrowserToolResult {
        guard let action = string(arguments["action"]) else {
            throw BrowserAutomationError.missingArgument("action")
        }

        switch action {
        case "openTab":
            let url = string(arguments["url"]) ?? store.settings.newTabURL
            let tab = store.newTab(url: url, select: true)
            return BrowserToolResult(text: "Opened tab \(tab.id.uuidString) at \(tab.urlString).", success: true)
        case "selectTab":
            guard let id = string(arguments["tabId"]) else {
                throw BrowserAutomationError.missingArgument("tabId")
            }
            let tab = try findTab(id, store: store)
            store.selectTab(tab.id)
            return BrowserToolResult(text: "Selected tab \(tab.id.uuidString): \(tab.title).", success: true)
        case "navigate":
            guard let url = string(arguments["url"]) else {
                throw BrowserAutomationError.missingArgument("url")
            }
            let tab = try targetTab(arguments: arguments, store: store)
            tab.load(url)
            return BrowserToolResult(text: "Navigating \(tab.id.uuidString) to \(tab.urlString).", success: true)
        case "back":
            try targetTab(arguments: arguments, store: store).goBack()
            return BrowserToolResult(text: "Went back.", success: true)
        case "forward":
            try targetTab(arguments: arguments, store: store).goForward()
            return BrowserToolResult(text: "Went forward.", success: true)
        case "reload":
            try targetTab(arguments: arguments, store: store).reload()
            return BrowserToolResult(text: "Reloaded the page.", success: true)
        case "readPage":
            let tab = try targetTab(arguments: arguments, store: store)
            let maxTextChars = int(arguments["maxTextChars"]) ?? 12_000
            let page = try await readPage(tab: tab, maxTextChars: maxTextChars, store: store)
            return BrowserToolResult(text: prettyJSON(page), success: true)
        case "click", "doubleClick", "hover", "hold", "type", "keyPress", "scroll":
            let tab = try targetTab(arguments: arguments, store: store)
            let result = try await runPageAction(action, arguments: arguments, tab: tab, store: store)
            return BrowserToolResult(text: prettyJSON(result), success: true)
        case "findText":
            guard let text = string(arguments["text"]) else {
                throw BrowserAutomationError.missingArgument("text")
            }
            try targetTab(arguments: arguments, store: store).find(text)
            return BrowserToolResult(text: "Finding text: \(text)", success: true)
        case "wait":
            let duration = int(arguments["durationMS"]) ?? 750
            try await Task.sleep(nanoseconds: UInt64(max(0, duration)) * 1_000_000)
            return BrowserToolResult(text: "Waited \(duration)ms.", success: true)
        default:
            throw BrowserAutomationError.unsupportedAction(action)
        }
    }

    @MainActor
    private static func getSettings(store: BrowserStore) throws -> String {
        let settings = store.settings
        let payload: [String: Any] = [
            "homepageURL": settings.homepageURL,
            "newTabBehavior": settings.newTabBehavior.rawValue,
            "searchEngine": settings.searchEngine.rawValue,
            "customSearchTemplate": settings.customSearchTemplate,
            "theme": settings.theme.rawValue,
            "showSidebarOnLaunch": settings.showSidebarOnLaunch,
            "sidebarPosition": settings.sidebarPosition.rawValue,
            "autoPiP": settings.autoPiP,
            "gradientTheme": settings.gradientTheme.isEmpty
                ? "none"
                : (settings.gradientTheme.presetID ?? "custom")
        ]
        return prettyJSON(payload)
    }

    @MainActor
    private static func updateSettings(arguments: [String: Any],
                                       store: BrowserStore) throws -> BrowserToolResult {
        let settings = store.settings
        var changes: [String] = []

        if let value = string(arguments["homepageURL"]) {
            settings.homepageURL = value
            changes.append("homepage → \(value)")
        }
        if let raw = string(arguments["newTabBehavior"]) {
            guard let value = NewTabBehavior(rawValue: raw) else {
                throw BrowserAutomationError.unsupportedAction("Unknown newTabBehavior: \(raw)")
            }
            settings.newTabBehavior = value
            changes.append("new-tab behavior → \(value.rawValue)")
        }
        if let raw = string(arguments["searchEngine"]) {
            guard let value = SearchEngine(rawValue: raw) else {
                throw BrowserAutomationError.unsupportedAction("Unknown searchEngine: \(raw)")
            }
            settings.searchEngine = value
            changes.append("search engine → \(value.rawValue)")
        }
        if let value = string(arguments["customSearchTemplate"]) {
            settings.customSearchTemplate = value
            changes.append("custom search template → \(value)")
        }
        if let raw = string(arguments["theme"]) {
            guard let value = ThemePreference(rawValue: raw) else {
                throw BrowserAutomationError.unsupportedAction("Unknown theme: \(raw)")
            }
            settings.theme = value
            changes.append("theme → \(value.rawValue)")
        }
        if let value = bool(arguments["showSidebarOnLaunch"]) {
            settings.showSidebarOnLaunch = value
            changes.append("show sidebar on launch → \(value)")
        }
        if let raw = string(arguments["sidebarPosition"]) {
            guard let value = SidebarPosition(rawValue: raw) else {
                throw BrowserAutomationError.unsupportedAction("Unknown sidebarPosition: \(raw)")
            }
            settings.sidebarPosition = value
            changes.append("sidebar position → \(value.rawValue)")
        }
        if let value = bool(arguments["autoPiP"]) {
            settings.autoPiP = value
            changes.append("auto Picture-in-Picture → \(value)")
        }
        if let raw = string(arguments["gradientTheme"]) {
            if raw == "none" {
                settings.gradientTheme = .none
                changes.append("gradient theme → none")
            } else if let preset = ThemePreset.all.first(where: { $0.id == raw }) {
                settings.gradientTheme = preset.theme
                changes.append("gradient theme → \(preset.name)")
            } else {
                throw BrowserAutomationError.unsupportedAction("Unknown gradientTheme: \(raw)")
            }
        }

        guard !changes.isEmpty else {
            return BrowserToolResult(
                text: "No settings were changed (no recognized fields provided).",
                success: false
            )
        }
        return BrowserToolResult(text: "Updated settings: " + changes.joined(separator: ", ") + ".", success: true)
    }

    @MainActor
    private static func targetTab(arguments: [String: Any], store: BrowserStore) throws -> BrowserTab {
        if let id = string(arguments["tabId"]) {
            return try findTab(id, store: store)
        }
        guard let tab = store.selectedTab else {
            throw BrowserAutomationError.browserUnavailable
        }
        return tab
    }

    @MainActor
    private static func findTab(_ id: String, store: BrowserStore) throws -> BrowserTab {
        if let tab = store.tabs.first(where: { $0.id.uuidString == id || $0.id.uuidString.hasPrefix(id) || String($0.extensionTabID) == id }) {
            return tab
        }
        throw BrowserAutomationError.tabNotFound(id)
    }

    private static func tabRecord(_ tab: BrowserTab) -> [String: Any] {
        [
            "id": tab.id.uuidString,
            "extensionTabId": tab.extensionTabID,
            "title": tab.title,
            "url": tab.urlString,
            "isLoading": tab.isLoading,
            "canGoBack": tab.canGoBack,
            "canGoForward": tab.canGoForward,
            "isRealized": tab.hasRealized
        ]
    }

    @MainActor
    private static func readPage(tab: BrowserTab, maxTextChars: Int, store: BrowserStore) async throws -> Any {
        try await waitForBrowser(tab, store: store)
        let source = """
        (() => {
          const max = \(max(500, maxTextChars));
          const clean = (value) => String(value || "").replace(/\\s+/g, " ").trim();
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
            selector: pathFor(el)
          }));
          const controls = Array.from(document.querySelectorAll("button,input,textarea,select,a,[role=button],[contenteditable=true]"))
            .filter(isVisible)
            .slice(0, 120)
            .map((el) => ({
              tag: el.localName,
              role: el.getAttribute("role") || "",
              type: el.getAttribute("type") || "",
              name: el.getAttribute("name") || "",
              text: clean(el.innerText || el.value || el.getAttribute("aria-label") || el.getAttribute("placeholder")).slice(0, 160),
              selector: pathFor(el),
              rect: (() => { const r = el.getBoundingClientRect(); return { x: Math.round(r.x), y: Math.round(r.y), width: Math.round(r.width), height: Math.round(r.height) }; })()
            }));
          return {
            title: document.title,
            url: location.href,
            selectedText: String(getSelection ? getSelection() : ""),
            visibleText: clean(document.body ? document.body.innerText : "").slice(0, max),
            links,
            controls,
            viewport: { width: innerWidth, height: innerHeight, devicePixelRatio },
            scroll: { x: scrollX, y: scrollY, maxY: Math.max(0, document.documentElement.scrollHeight - innerHeight) }
          };
        })()
        """
        return try await tab.evaluateJavaScript(source)
    }

    @MainActor
    private static func runPageAction(_ action: String,
                                      arguments: [String: Any],
                                      tab: BrowserTab,
                                      store: BrowserStore) async throws -> Any {
        try await waitForBrowser(tab, store: store)
        let selector = jsLiteral(string(arguments["selector"]) ?? "")
        let text = jsLiteral(string(arguments["text"]) ?? "")
        let key = jsLiteral(string(arguments["key"]) ?? "")
        let direction = jsLiteral(string(arguments["direction"]) ?? "down")
        let x = number(arguments["x"]) ?? -1
        let y = number(arguments["y"]) ?? -1
        let amount = number(arguments["amount"]) ?? 600
        let duration = int(arguments["durationMS"]) ?? 450
        let source = """
        (async () => {
          const action = \(jsLiteral(action));
          const selector = \(selector);
          const text = \(text);
          const key = \(key);
          const direction = \(direction);
          const x = \(x);
          const y = \(y);
          const amount = \(amount);
          const duration = \(max(0, duration));
          const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
          const clean = (value) => String(value || "").replace(/\\s+/g, " ").trim();
          const target = () => {
            if (selector) {
              const el = document.querySelector(selector);
              if (el) {
                try { el.scrollIntoView({ block: "center", inline: "center" }); } catch (e) {}
              }
              return el;
            }
            if (x >= 0 && y >= 0) return document.elementFromPoint(x, y);
            return document.activeElement || document.body;
          };
          const describe = (el) => {
            if (!el) return { found: false };
            const r = el.getBoundingClientRect();
            return {
              found: true,
              tag: el.localName,
              id: el.id || "",
              text: clean(el.innerText || el.value || el.getAttribute("aria-label") || "").slice(0, 160),
              rect: { x: Math.round(r.x), y: Math.round(r.y), width: Math.round(r.width), height: Math.round(r.height) }
            };
          };
          const mouse = (el, type, detail = 1) => {
            const r = el.getBoundingClientRect();
            const cx = x >= 0 ? x : r.left + r.width / 2;
            const cy = y >= 0 ? y : r.top + r.height / 2;
            el.dispatchEvent(new MouseEvent(type, { bubbles: true, cancelable: true, view: window, clientX: cx, clientY: cy, detail }));
          };
          const el = target();
          if (["click", "doubleClick", "hover", "hold", "type"].includes(action) && !el) {
            throw new Error("No target element matched the selector or coordinates.");
          }
          if (action === "click" || action === "doubleClick") {
            mouse(el, "mousemove");
            mouse(el, "mousedown");
            if (typeof el.focus === "function") el.focus();
            mouse(el, "mouseup");
            mouse(el, "click");
            if (action === "doubleClick") mouse(el, "dblclick", 2);
            return { action, target: describe(el), url: location.href };
          }
          if (action === "hover") {
            mouse(el, "mousemove");
            mouse(el, "mouseover");
            return { action, target: describe(el) };
          }
          if (action === "hold") {
            mouse(el, "mousedown");
            await sleep(duration);
            mouse(el, "mouseup");
            return { action, durationMS: duration, target: describe(el) };
          }
          if (action === "type") {
            if (typeof el.focus === "function") el.focus();
            if ("value" in el) {
              const start = Number.isFinite(el.selectionStart) ? el.selectionStart : String(el.value || "").length;
              const end = Number.isFinite(el.selectionEnd) ? el.selectionEnd : start;
              const current = String(el.value || "");
              el.value = current.slice(0, start) + text + current.slice(end);
              const cursor = start + text.length;
              if (typeof el.setSelectionRange === "function") el.setSelectionRange(cursor, cursor);
              el.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: text }));
              el.dispatchEvent(new Event("change", { bubbles: true }));
            } else {
              document.execCommand("insertText", false, text);
            }
            return { action, target: describe(el) };
          }
          if (action === "keyPress") {
            const active = document.activeElement || document.body;
            active.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
            active.dispatchEvent(new KeyboardEvent("keyup", { key, bubbles: true, cancelable: true }));
            return { action, key, target: describe(active) };
          }
          if (action === "scroll") {
            const dx = direction === "left" ? -amount : (direction === "right" ? amount : 0);
            const dy = direction === "up" ? -amount : (direction === "down" ? amount : 0);
            if (selector && el) {
              el.scrollBy({ left: dx, top: dy, behavior: "smooth" });
            } else {
              window.scrollBy({ left: dx, top: dy, behavior: "smooth" });
            }
            await sleep(120);
            return { action, scroll: { x: scrollX, y: scrollY }, target: selector && el ? describe(el) : null };
          }
          throw new Error("Unsupported page action: " + action);
        })()
        """
        return try await tab.evaluateJavaScript(source)
    }

    @MainActor
    private static func waitForBrowser(_ tab: BrowserTab, store: BrowserStore) async throws {
        if tab.browserView.browserIdentifier != 0 { return }

        // `realize()` only flips a (non-@Published) flag. The CEF browser is
        // created lazily by `WebContainerView.updateNSView` once the view is
        // mounted in the window with a real size — and that mount happens only on
        // a SwiftUI render. So realizing a fresh/background tab is not enough on
        // its own: without a published change, the view never mounts, the browser
        // is never created, and we'd time out no matter how long we wait. Nudge
        // the store so the container re-renders and mounts the tab, then let the
        // run loop service the render plus CEF's async OnAfterCreated callback.
        _ = tab.realize()
        store.objectWillChange.send()

        // ~6s budget; cold start (first browser, web area still sizing up) and
        // slow tab mounts can take noticeably longer than the steady-state case.
        for attempt in 0..<60 {
            if tab.browserView.browserIdentifier != 0 { return }
            // Re-nudge periodically in case the first render landed before the
            // web container had non-zero bounds (creation bails until it does).
            if attempt % 5 == 4 { store.objectWillChange.send() }
            try await Task.sleep(nanoseconds: 100_000_000)
        }
        throw BrowserAutomationError.browserUnavailable
    }

    private static func prettyJSON(_ value: Any) -> String {
        let safe = jsonReady(value)
        guard JSONSerialization.isValidJSONObject(safe),
              let data = try? JSONSerialization.data(withJSONObject: safe, options: [.prettyPrinted, .sortedKeys]),
              let text = String(data: data, encoding: .utf8)
        else {
            return String(describing: value)
        }
        return text
    }

    private static func jsonReady(_ value: Any) -> Any {
        switch value {
        case let dict as [String: Any]:
            return dict.mapValues(jsonReady)
        case let dict as NSDictionary:
            var out: [String: Any] = [:]
            dict.forEach { key, value in out[String(describing: key)] = jsonReady(value) }
            return out
        case let array as [Any]:
            return array.map(jsonReady)
        case let array as NSArray:
            return array.map(jsonReady)
        case let number as NSNumber:
            return number
        case let string as String:
            return string
        case is NSNull:
            return NSNull()
        default:
            return String(describing: value)
        }
    }

    private static func jsLiteral(_ string: String) -> String {
        guard let data = try? JSONSerialization.data(withJSONObject: [string], options: []),
              let array = String(data: data, encoding: .utf8),
              array.count >= 2
        else {
            return "\"\""
        }
        return String(array.dropFirst().dropLast())
    }

    private static func string(_ value: Any?) -> String? {
        if let string = value as? String { return string }
        if let value = value { return String(describing: value) }
        return nil
    }

    private static func bool(_ value: Any?) -> Bool? {
        if let bool = value as? Bool { return bool }
        if let number = value as? NSNumber { return number.boolValue }
        if let string = value as? String { return Bool(string) }
        return nil
    }

    private static func int(_ value: Any?) -> Int? {
        if let int = value as? Int { return int }
        if let number = value as? NSNumber { return number.intValue }
        if let string = value as? String { return Int(string) }
        return nil
    }

    private static func number(_ value: Any?) -> Double? {
        if let double = value as? Double { return double }
        if let number = value as? NSNumber { return number.doubleValue }
        if let string = value as? String { return Double(string) }
        return nil
    }
}
