import SwiftUI
import AppKit
import CryptoKit

/// One installed Chrome extension (an unpacked extension directory the user
/// imported). `path` points at Mori's *managed* copy under Application
/// Support, so the original folder can move or be deleted without breaking it.
struct BrowserExtension: Identifiable, Codable, Equatable {
    let id: String          // Chrome extension id when known, else a local UUID
    var name: String
    var version: String
    var detail: String      // manifest "description"
    var path: String        // absolute path to the unpacked extension directory
    var iconPath: String?   // absolute path to the best-resolution icon, if any
    var popupPage: String?  // action.default_popup, relative to the extension
    var optionsPage: String? // options_ui.page / options_page, relative
    var enabled: Bool
    var pinned: Bool        // surfaced as its own icon in the omnibox

    init(id: String, name: String, version: String, detail: String,
         path: String, iconPath: String?, popupPage: String? = nil,
         optionsPage: String? = nil, enabled: Bool, pinned: Bool = false) {
        self.id = id
        self.name = name
        self.version = version
        self.detail = detail
        self.path = path
        self.iconPath = iconPath
        self.popupPage = popupPage
        self.optionsPage = optionsPage
        self.enabled = enabled
        self.pinned = pinned
    }

    // Custom decode so catalogs written by an earlier build (without the
    // pin/action fields) still load, defaulting the new fields.
    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        name = try c.decode(String.self, forKey: .name)
        version = try c.decode(String.self, forKey: .version)
        detail = try c.decode(String.self, forKey: .detail)
        path = try c.decode(String.self, forKey: .path)
        iconPath = try c.decodeIfPresent(String.self, forKey: .iconPath)
        popupPage = try c.decodeIfPresent(String.self, forKey: .popupPage)
        optionsPage = try c.decodeIfPresent(String.self, forKey: .optionsPage)
        enabled = try c.decode(Bool.self, forKey: .enabled)
        pinned = try c.decodeIfPresent(Bool.self, forKey: .pinned) ?? false
    }
}

/// Manages the set of Chrome extensions Mori understands. The visible browser
/// is an embedded Chromium surface, so Mori parses manifests and injects
/// supported content scripts itself instead of delegating to Chrome's windowed
/// extension runtime. Changes apply on the next page load/reload.
///
/// Persistence is twofold:
///   • the full catalog (with metadata) as JSON under `Key.catalog`, and
///   • the bare list of *enabled* absolute paths under `Key.enabledPaths`,
///     retained so older builds can still read the catalog shape.
final class ExtensionStore: ObservableObject {
    static let shared = ExtensionStore()

    struct ActionState: Equatable {
        var badgeText: String = ""
        var badgeBackgroundColor: [Int] = [217, 48, 37, 255]
        var badgeTextColor: [Int] = [255, 255, 255, 255]
        var title: String?
        var popupPage: String?
        var isEnabled: Bool = true
    }

    struct CommandDescriptor: Equatable {
        let extensionID: String
        let extensionName: String
        let commandName: String
        let description: String
        let shortcut: String
    }

    @Published private(set) var extensions: [BrowserExtension] = []
    @Published private(set) var actionStates: [String: ActionState] = [:]
    /// Surfaced to the UI when an import fails (bad folder, missing manifest…).
    @Published var lastError: String?
    /// Web Store ids with an install in flight, so the UI can show progress.
    @Published private(set) var installingIDs: Set<String> = []

    private let defaults: UserDefaults
    private let catalogIsEphemeral: Bool
    private var uninstallURLs: [String: String] = [:]

    private enum Key {
        static let catalog = "mori.extensions"
        // Kept for catalog compatibility with earlier builds.
        static let enabledPaths = "mori.enabledExtensionPaths"
        static let uninstallURLs = "mori.extensionUninstallURLs"
    }

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        let environmentCatalog = ProcessInfo.processInfo.environment["MORI_EXTENSION_CATALOG_JSON"]
        catalogIsEphemeral = environmentCatalog != nil
        uninstallURLs = defaults.dictionary(forKey: Key.uninstallURLs) as? [String: String] ?? [:]

        if let environmentCatalog,
           let data = environmentCatalog.data(using: .utf8),
           let decoded = try? JSONDecoder().decode([BrowserExtension].self, from: data) {
            extensions = decoded.filter {
                FileManager.default.fileExists(atPath: $0.path)
            }
        } else if let data = defaults.data(forKey: Key.catalog),
           let decoded = try? JSONDecoder().decode([BrowserExtension].self, from: data) {
            // Drop any whose managed directory has vanished (e.g. cleared cache).
            extensions = decoded.filter {
                FileManager.default.fileExists(atPath: $0.path)
            }
            if extensions.count != decoded.count { persist() }
        }
    }

    // MARK: - Mutations

    /// Toggle an extension on/off. Takes effect on the next page load/reload.
    func setEnabled(_ ext: BrowserExtension, _ enabled: Bool) {
        guard let idx = extensions.firstIndex(where: { $0.id == ext.id }) else { return }
        extensions[idx].enabled = enabled
        persist()
    }

    /// Remove an extension and delete its managed copy from disk.
    func remove(_ ext: BrowserExtension) {
        extensions.removeAll { $0.id == ext.id }
        actionStates.removeValue(forKey: ext.id)
        let uninstallURL = uninstallURLs.removeValue(forKey: ext.id)
        try? FileManager.default.removeItem(atPath: ext.path)
        persist()
        persistUninstallURLs()
        if let uninstallURL, !uninstallURL.isEmpty {
            NotificationCenter.default.post(name: .moriOpenExtensionUninstallURL,
                                            object: nil,
                                            userInfo: ["url": uninstallURL])
        }
    }

    /// Present a folder picker and import the chosen unpacked extension.
    func presentImportPanel() {
        let panel = NSOpenPanel()
        panel.title = "Add Extension"
        panel.message = "Choose an unpacked extension folder (the directory containing manifest.json)."
        panel.prompt = "Add"
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.allowsMultipleSelection = false
        panel.canCreateDirectories = false
        if panel.runModal() == .OK, let url = panel.url {
            importExtension(fromUnpackedFolder: url)
        }
    }

    /// Validate, copy into the managed directory, parse, and register an
    /// unpacked extension folder. Sets `lastError` on failure.
    func importExtension(fromUnpackedFolder source: URL) {
        let fm = FileManager.default
        let manifestURL = source.appendingPathComponent("manifest.json")
        guard fm.fileExists(atPath: manifestURL.path) else {
            lastError = "That folder has no manifest.json — pick an unpacked extension directory."
            return
        }

        let id = UUID().uuidString
        let dest = Self.managedDirectory().appendingPathComponent(id, isDirectory: true)
        do {
            try fm.createDirectory(at: Self.managedDirectory(),
                                   withIntermediateDirectories: true)
            try fm.copyItem(at: source, to: dest)
        } catch {
            lastError = "Couldn't copy the extension: \(error.localizedDescription)"
            return
        }

        guard let meta = Self.readManifest(at: dest) else {
            try? fm.removeItem(at: dest)
            lastError = "Couldn't read manifest.json in that folder."
            return
        }
        register(directory: dest, meta: meta, preferredID: meta.manifestKeyExtensionID)
        lastError = nil
    }

    /// Build, append, and persist an extension record for an already-unpacked
    /// managed directory. Web Store installs keep their real Chrome extension id;
    /// local unpacked imports use the managed folder name as their stable id.
    @discardableResult
    private func register(directory dir: URL,
                          meta: ManifestMeta,
                          preferredID: String? = nil) -> BrowserExtension {
        let id = preferredID ?? dir.lastPathComponent
        let existing = extensions.first { $0.id == id }
        let ext = BrowserExtension(
            id: id,
            name: meta.name,
            version: meta.version,
            detail: meta.detail,
            path: dir.path,
            iconPath: meta.iconPath,
            popupPage: meta.popupPage,
            optionsPage: meta.optionsPage,
            enabled: existing?.enabled ?? true,
            pinned: existing?.pinned ?? false)
        if let existingIndex = extensions.firstIndex(where: { $0.id == id }) {
            let previousPath = extensions[existingIndex].path
            extensions[existingIndex] = ext
            if previousPath != ext.path {
                try? FileManager.default.removeItem(atPath: previousPath)
            }
        } else {
            extensions.append(ext)
        }
        persist()
        return ext
    }

    /// Pin/unpin an extension so it shows as its own omnibox icon.
    func togglePinned(_ ext: BrowserExtension) {
        guard let idx = extensions.firstIndex(where: { $0.id == ext.id }) else { return }
        extensions[idx].pinned.toggle()
        persist()
    }

    /// Pinned, currently-enabled extensions, in install order.
    var pinnedExtensions: [BrowserExtension] {
        extensions.filter { $0.pinned && $0.enabled }
    }

    func actionState(for extensionID: String) -> ActionState {
        actionStates[extensionID] ?? ActionState()
    }

    @discardableResult
    func requestActionPopup(extensionID: String, reason: String) -> Bool {
        guard let ext = extensions.first(where: { $0.id == extensionID }),
              popupURL(for: ext) != nil
        else { return false }
        DispatchQueue.main.async {
            NotificationCenter.default.post(
                name: .moriOpenExtensionPopup,
                object: nil,
                userInfo: [
                    "extensionId": extensionID,
                    "reason": reason
                ]
            )
        }
        return true
    }

    func handleAction(method: String,
                      args: NSDictionary,
                      extensionID: String) -> NSDictionary {
        let details = args["details"] as? NSDictionary ?? [:]
        var state = actionStates[extensionID] ?? ActionState()

        switch method {
        case "action.setBadgeText":
            state.badgeText = details["text"] as? String ?? ""
            actionStates[extensionID] = state
            return ["result": NSNull()]
        case "action.getBadgeText":
            return ["result": state.badgeText]
        case "action.setBadgeBackgroundColor":
            state.badgeBackgroundColor = Self.badgeColor(from: details["color"])
                ?? state.badgeBackgroundColor
            actionStates[extensionID] = state
            return ["result": NSNull()]
        case "action.getBadgeBackgroundColor":
            return ["result": state.badgeBackgroundColor]
        case "action.setBadgeTextColor":
            state.badgeTextColor = Self.badgeColor(from: details["color"])
                ?? state.badgeTextColor
            actionStates[extensionID] = state
            return ["result": NSNull()]
        case "action.getBadgeTextColor":
            return ["result": state.badgeTextColor]
        case "action.setTitle":
            state.title = details["title"] as? String
            actionStates[extensionID] = state
            return ["result": NSNull()]
        case "action.getTitle":
            let fallback = extensions.first { $0.id == extensionID }?.name ?? ""
            return ["result": state.title ?? fallback]
        case "action.setPopup":
            state.popupPage = details["popup"] as? String
            actionStates[extensionID] = state
            return ["result": NSNull()]
        case "action.getPopup":
            let fallback = extensions.first { $0.id == extensionID }?.popupPage
            let popup = state.popupPage ?? fallback ?? ""
            if popup.isEmpty {
                return ["result": ""]
            }
            return ["result": Self.extensionResourceURL(extensionID: extensionID, path: popup)]
        case "action.enable":
            state.isEnabled = true
            actionStates[extensionID] = state
            return ["result": NSNull()]
        case "action.disable":
            state.isEnabled = false
            actionStates[extensionID] = state
            return ["result": NSNull()]
        case "action.isEnabled":
            return ["result": state.isEnabled]
        case "action.openPopup":
            if !requestActionPopup(extensionID: extensionID, reason: "api") {
                return ["error": "This extension does not have an enabled popup."]
            }
            return ["result": NSNull()]
        case "action.getUserSettings":
            let isOnToolbar = extensions.first { $0.id == extensionID }?.pinned ?? false
            return ["result": ["isOnToolbar": isOnToolbar]]
        case "action.setIcon":
            return ["result": NSNull()]
        default:
            return ["error": "Unsupported action method: \(method)"]
        }
    }

    func handleManagement(method: String,
                          args: NSDictionary,
                          extensionID: String) -> NSDictionary {
        switch method {
        case "management.getSelf":
            guard let ext = installedExtension(withID: extensionID) else {
                return ["error": "Extension is not installed."]
            }
            return ["result": managementInfo(for: ext)]
        case "management.get":
            guard let id = args["id"] as? String, !id.isEmpty else {
                return ["error": "Missing extension id."]
            }
            guard let ext = installedExtension(withID: id) else {
                return ["result": NSNull()]
            }
            return ["result": managementInfo(for: ext)]
        case "management.getAll":
            return ["result": extensions.map { managementInfo(for: $0) }]
        case "management.setEnabled":
            guard let id = args["id"] as? String, !id.isEmpty else {
                return ["error": "Missing extension id."]
            }
            guard let enabled = args["enabled"] as? NSNumber else {
                return ["error": "Missing enabled flag."]
            }
            guard let ext = installedExtension(withID: id) else {
                return ["error": "Extension is not installed."]
            }
            setEnabled(ext, enabled.boolValue)
            return ["result": NSNull()]
        case "management.uninstall":
            guard let id = args["id"] as? String, !id.isEmpty else {
                return ["error": "Missing extension id."]
            }
            guard id.caseInsensitiveCompare(extensionID) != .orderedSame else {
                return ["error": "Use management.uninstallSelf to remove the calling extension."]
            }
            guard let ext = installedExtension(withID: id) else {
                return ["error": "Extension is not installed."]
            }
            remove(ext)
            return ["result": NSNull()]
        case "management.uninstallSelf":
            guard let ext = installedExtension(withID: extensionID) else {
                return ["error": "Extension is not installed."]
            }
            remove(ext)
            return ["result": NSNull()]
        default:
            return ["error": "Unsupported management method: \(method)"]
        }
    }

    func setUninstallURL(forExtensionID extensionID: String, url rawURL: String) -> NSDictionary {
        guard installedExtension(withID: extensionID) != nil else {
            return ["error": "Extension is not installed."]
        }
        let trimmed = rawURL.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.isEmpty {
            uninstallURLs.removeValue(forKey: extensionID)
            persistUninstallURLs()
            return ["result": NSNull()]
        }
        guard trimmed.count <= 1023 else {
            return ["error": "Uninstall URL is too long."]
        }
        guard let components = URLComponents(string: trimmed),
              let scheme = components.scheme?.lowercased(),
              (scheme == "http" || scheme == "https"),
              components.host?.isEmpty == false
        else {
            return ["error": "Uninstall URL must be an http or https URL."]
        }
        uninstallURLs[extensionID] = trimmed
        persistUninstallURLs()
        return ["result": NSNull()]
    }

    func commands() -> [CommandDescriptor] {
        extensions.flatMap { ext -> [CommandDescriptor] in
            guard ext.enabled,
                  let manifest = Self.readManifestJSON(at: URL(fileURLWithPath: ext.path)),
                  let commands = manifest["commands"] as? [String: Any]
            else { return [] }

            return commands.compactMap { name, raw in
                guard let details = raw as? [String: Any],
                      let shortcut = Self.commandShortcut(in: details),
                      !shortcut.isEmpty
                else { return nil }
                return CommandDescriptor(
                    extensionID: ext.id,
                    extensionName: ext.name,
                    commandName: name,
                    description: details["description"] as? String ?? "",
                    shortcut: shortcut)
            }
        }
    }

    private func installedExtension(withID id: String) -> BrowserExtension? {
        extensions.first { $0.id.caseInsensitiveCompare(id) == .orderedSame }
    }

    private func managementInfo(for ext: BrowserExtension) -> [String: Any] {
        let manifest = Self.readManifestJSON(at: URL(fileURLWithPath: ext.path)) ?? [:]
        let shortName = manifest["short_name"] as? String
        let offlineEnabled = (manifest["offline_enabled"] as? NSNumber)?.boolValue ?? false
        let optionsURL = optionsURL(for: ext) ?? ""
        var info: [String: Any] = [
            "id": ext.id,
            "name": ext.name,
            "shortName": shortName ?? ext.name,
            "description": ext.detail,
            "version": ext.version,
            "enabled": ext.enabled,
            "type": "extension",
            "installType": "development",
            "mayDisable": true,
            "offlineEnabled": offlineEnabled,
            "optionsUrl": optionsURL
        ]
        if let iconPath = ext.iconPath {
            info["icons"] = [["size": 128, "url": URL(fileURLWithPath: iconPath).absoluteString]]
        }
        return info
    }

    private static func badgeColor(from raw: Any?) -> [Int]? {
        let rawArray: [Any]?
        if let array = raw as? [Any] {
            rawArray = array
        } else if let array = raw as? NSArray {
            rawArray = array.compactMap { $0 }
        } else {
            rawArray = nil
        }

        if let array = rawArray, array.count >= 3 {
            let values = array.prefix(4).compactMap { item -> Int? in
                if let number = item as? NSNumber { return number.intValue }
                if let string = item as? String { return Int(string) }
                return nil
            }
            guard values.count >= 3 else { return nil }
            let alpha = values.count >= 4 ? values[3] : 255
            return [
                min(max(values[0], 0), 255),
                min(max(values[1], 0), 255),
                min(max(values[2], 0), 255),
                min(max(alpha, 0), 255)
            ]
        }

        guard let string = raw as? String else { return nil }
        let text = string.trimmingCharacters(in: .whitespacesAndNewlines)
        guard text.hasPrefix("#") else { return nil }
        let hex = String(text.dropFirst())
        let scanner = Scanner(string: hex)
        var value: UInt64 = 0
        guard scanner.scanHexInt64(&value) else { return nil }
        switch hex.count {
        case 6:
            return [
                Int((value >> 16) & 0xff),
                Int((value >> 8) & 0xff),
                Int(value & 0xff),
                255
            ]
        case 8:
            return [
                Int((value >> 24) & 0xff),
                Int((value >> 16) & 0xff),
                Int((value >> 8) & 0xff),
                Int(value & 0xff)
            ]
        default:
            return nil
        }
    }

    func command(matching event: NSEvent) -> CommandDescriptor? {
        command(matching: MoriShortcutTrigger(event: event))
    }

    func command(matching trigger: MoriShortcutTrigger) -> CommandDescriptor? {
        commands().first { Self.shortcut($0.shortcut, matches: trigger) }
    }

    private static func commandShortcut(in details: [String: Any]) -> String? {
        guard let suggested = details["suggested_key"] else { return nil }
        if let shortcut = suggested as? String { return shortcut }
        guard let byPlatform = suggested as? [String: Any] else { return nil }
        if let mac = byPlatform["mac"] as? String { return mac }
        if let chromeOS = byPlatform["chromeos"] as? String { return chromeOS }
        if let fallback = byPlatform["default"] as? String { return fallback }
        return nil
    }

    private static func shortcut(_ shortcut: String, matches event: NSEvent) -> Bool {
        Self.shortcut(shortcut, matches: MoriShortcutTrigger(event: event))
    }

    private static func shortcut(_ shortcut: String, matches trigger: MoriShortcutTrigger) -> Bool {
        var required = NSEvent.ModifierFlags()
        var key: String?

        for rawToken in shortcut.split(separator: "+") {
            let token = rawToken.trimmingCharacters(in: .whitespacesAndNewlines)
            switch token.lowercased() {
            case "command", "cmd", "meta", "commandorcontrol":
                required.insert(.command)
            case "ctrl", "control":
                // Chrome's cross-platform "Ctrl" command maps to Command on macOS.
                required.insert(.command)
            case "macctrl":
                required.insert(.control)
            case "alt", "option":
                required.insert(.option)
            case "shift":
                required.insert(.shift)
            default:
                key = normalizedCommandKey(token)
            }
        }

        guard let key else { return false }
        return trigger.modifiers == required && trigger.key == key
    }

    private static func normalizedCommandKey(_ raw: String) -> String {
        MoriShortcutTrigger.normalizedPrintableKey(raw)
    }

    struct BackgroundRunner: Identifiable, Equatable {
        let id: String
        let url: String
    }

    /// Enabled extensions with a background/event script surface Mori should
    /// keep alive in a hidden Chromium view.
    var backgroundRunners: [BackgroundRunner] {
        extensions.compactMap { ext in
            guard ext.enabled, let url = backgroundURL(for: ext) else { return nil }
            return BackgroundRunner(id: ext.id, url: url)
        }
    }

    /// The browser-action popup page, hosted by Mori chrome instead of a user tab.
    func popupURL(for ext: BrowserExtension) -> String? {
        guard ext.enabled,
              actionStates[ext.id]?.isEnabled ?? true,
              let popup = actionStates[ext.id]?.popupPage ?? ext.popupPage,
              !popup.isEmpty
        else { return nil }
        return Self.extensionResourceURL(extensionID: ext.id, path: popup)
    }

    func popupURL(forExtensionID extensionID: String) -> String? {
        guard let ext = extensions.first(where: { $0.id == extensionID }) else {
            return nil
        }
        return popupURL(for: ext)
    }

    /// Chrome sizes action popups to the extension page. Use declared CSS
    /// dimensions when we can read them, then clamp to browser-popup bounds.
    func popupSize(for ext: BrowserExtension) -> CGSize {
        let fallback = CGSize(width: 420, height: 520)
        guard let popup = actionStates[ext.id]?.popupPage ?? ext.popupPage,
              !popup.isEmpty
        else {
            return fallback
        }
        let popupURL = URL(fileURLWithPath: ext.path).appendingPathComponent(popup)
        guard let html = try? String(contentsOf: popupURL, encoding: .utf8) else {
            return fallback
        }

        let width = Self.cssPixelValue(named: "--popup-width", in: html)
            ?? Self.cssPixelValue(property: "width", in: html)
            ?? fallback.width
        let height = Self.cssPixelValue(named: "--popup-height", in: html)
            ?? Self.cssPixelValue(property: "height", in: html)
            ?? fallback.height

        return CGSize(
            width: min(max(width, 240), 800),
            height: min(max(height, 160), 600)
        )
    }

    /// The options page, which is normal extension content and may open as a tab.
    func optionsURL(for ext: BrowserExtension) -> String? {
        guard ext.enabled,
              let page = ext.optionsPage,
              !page.isEmpty
        else { return nil }
        return Self.extensionResourceURL(extensionID: ext.id, path: page)
    }

    func optionsURL(forExtensionID extensionID: String) -> String? {
        guard let ext = extensions.first(where: { $0.id == extensionID }) else {
            return nil
        }
        return optionsURL(for: ext)
    }

    func sidePanelURL(forExtensionID extensionID: String) -> String? {
        guard let ext = extensions.first(where: { $0.id == extensionID && $0.enabled }) else {
            return nil
        }
        let manifestURL = URL(fileURLWithPath: ext.path).appendingPathComponent("manifest.json")
        guard let data = try? Data(contentsOf: manifestURL),
              let manifest = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let sidePanel = manifest["side_panel"] as? [String: Any],
              let path = sidePanel["default_path"] as? String,
              !path.isEmpty
        else { return nil }
        return Self.extensionResourceURL(extensionID: ext.id, path: path)
    }

    func extensionResourceURL(forExtensionID extensionID: String, path: String) -> String {
        Self.extensionResourceURL(extensionID: extensionID, path: path)
    }

    private static func extensionResourceURL(extensionID: String, path: String) -> String {
        var allowed = CharacterSet.urlPathAllowed
        allowed.remove(charactersIn: "#?")
        let encoded = path.split(separator: "/", omittingEmptySubsequences: false)
            .map { String($0).addingPercentEncoding(withAllowedCharacters: allowed) ?? String($0) }
            .joined(separator: "/")
        return "mori-extension://\(extensionID)/\(encoded)"
    }

    private static func cssPixelValue(named variable: String, in text: String) -> CGFloat? {
        cssPixelValue(pattern: "\(NSRegularExpression.escapedPattern(for: variable))\\s*:\\s*([0-9]+(?:\\.[0-9]+)?)px", in: text)
    }

    private static func cssPixelValue(property: String, in text: String) -> CGFloat? {
        cssPixelValue(pattern: "\\b\(NSRegularExpression.escapedPattern(for: property))\\s*:\\s*var\\([^)]*,\\s*([0-9]+(?:\\.[0-9]+)?)px\\)", in: text)
            ?? cssPixelValue(pattern: "\\b\(NSRegularExpression.escapedPattern(for: property))\\s*:\\s*([0-9]+(?:\\.[0-9]+)?)px", in: text)
    }

    private static func cssPixelValue(pattern: String, in text: String) -> CGFloat? {
        guard let regex = try? NSRegularExpression(
            pattern: pattern,
            options: [.caseInsensitive]
        ) else { return nil }
        let range = NSRange(text.startIndex..<text.endIndex, in: text)
        guard let match = regex.firstMatch(in: text, range: range),
              match.numberOfRanges > 1,
              let valueRange = Range(match.range(at: 1), in: text),
              let value = Double(text[valueRange])
        else { return nil }
        return CGFloat(value)
    }

    func backgroundURL(for ext: BrowserExtension) -> String? {
        let manifestURL = URL(fileURLWithPath: ext.path).appendingPathComponent("manifest.json")
        guard let data = try? Data(contentsOf: manifestURL),
              let manifest = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let background = manifest["background"] as? [String: Any]
        else { return nil }

        if let page = background["page"] as? String, !page.isEmpty {
            return Self.extensionResourceURL(extensionID: ext.id, path: page)
        }
        if let worker = background["service_worker"] as? String, !worker.isEmpty {
            return "mori-extension://\(ext.id)/__mori_background__.html"
        }
        if let scripts = background["scripts"] as? [String], !scripts.isEmpty {
            return "mori-extension://\(ext.id)/__mori_background__.html"
        }
        return nil
    }

    func runtimeContexts(forExtensionID extensionID: String, filter: NSDictionary) -> [[String: Any]] {
        guard let ext = extensions.first(where: { $0.id.caseInsensitiveCompare(extensionID) == .orderedSame && $0.enabled }) else {
            return []
        }

        let contextTypes = Self.stringArray(from: filter["contextTypes"])
        let contextIDs = Self.stringArray(from: filter["contextIds"])
        let documentURLs = Self.stringArray(from: filter["documentUrls"])
        if let incognito = filter["incognito"] as? NSNumber, incognito.boolValue {
            return []
        }

        func allowed(type: String, id: String, documentURL: String) -> Bool {
            if let contextTypes, !contextTypes.contains(type) { return false }
            if let contextIDs, !contextIDs.contains(id) { return false }
            if let documentURLs, !documentURLs.contains(documentURL) { return false }
            return true
        }

        var contexts: [[String: Any]] = []
        if let url = backgroundURL(for: ext) {
            let id = "background:\(ext.id)"
            if allowed(type: "BACKGROUND", id: id, documentURL: url) {
                contexts.append([
                    "contextId": id,
                    "contextType": "BACKGROUND",
                    "documentUrl": url,
                    "frameId": 0,
                    "incognito": false
                ])
            }
        }
        return contexts
    }

    private static func stringArray(from value: Any?) -> [String]? {
        guard let array = value as? [Any] else { return nil }
        return array.compactMap { $0 as? String }
    }

    // MARK: - CRX install (Web Store / .crx files)

    /// Download an extension by its Chrome Web Store id, unpack, and install it.
    /// Shows a native confirmation on success, or an error alert on failure.
    /// Idempotent per id while a download is in flight.
    func beginWebStoreInstall(extensionID id: String) {
        guard !installingIDs.contains(id) else { return }
        installingIDs.insert(id)
        Task { @MainActor in
            defer { installingIDs.remove(id) }
            do {
                let data = try await downloadCRX(extensionID: id)
                let ext = try installCRXData(data, name: id)
                presentInstalledAlert(ext)
            } catch {
                presentInstallError(error)
            }
        }
    }

    /// Unpack a CRX payload into the managed directory and register it. Must run
    /// on the main actor (mutates published state); the unzip itself is quick.
    @MainActor
    @discardableResult
    fileprivate func installCRXData(_ data: Data, name: String) throws -> BrowserExtension {
        let package = try Self.crxPayload(from: data)
        let filenameID = Self.chromeExtensionIDCandidate(from: name)
        if let filenameID, let packageID = package.extensionID, filenameID != packageID {
            throw ExtensionInstallError.extensionIDMismatch(expected: filenameID, actual: packageID)
        }
        let preferredID = package.extensionID ?? filenameID
        let dir = try Self.unpack(zipData: package.zip)
        guard let meta = Self.readManifest(at: dir) else {
            try? FileManager.default.removeItem(at: dir)
            throw ExtensionInstallError.noManifest
        }
        lastError = nil
        return register(directory: dir,
                        meta: meta,
                        preferredID: preferredID ?? meta.manifestKeyExtensionID)
    }

    /// Fetch the CRX bytes for an extension id from Google's update service.
    private func downloadCRX(extensionID id: String) async throws -> Data {
        // The `x` parameter packs its own key=value&… payload, so its reserved
        // characters must be percent-encoded to survive as a single query value.
        let payload = "id=\(id)&installsource=ondemand&uc"
        let xEncoded = payload.addingPercentEncoding(withAllowedCharacters: .alphanumerics) ?? payload
        let urlString =
            "https://clients2.google.com/service/update2/crx?response=redirect"
            + "&acceptformat=crx2,crx3&prodversion=\(Self.chromeProdVersion)&x=\(xEncoded)"
        guard let url = URL(string: urlString) else {
            throw ExtensionInstallError.download("Invalid request URL.")
        }
        var request = URLRequest(url: url)
        request.setValue(Self.chromeUserAgent, forHTTPHeaderField: "User-Agent")
        let (data, response) = try await URLSession.shared.data(for: request)
        if let http = response as? HTTPURLResponse, !(200...299).contains(http.statusCode) {
            throw ExtensionInstallError.download("The store returned HTTP \(http.statusCode).")
        }
        guard !data.isEmpty else {
            throw ExtensionInstallError.download("The store returned an empty response.")
        }
        return data
    }

    // MARK: - CRX unpacking

    private struct CRXPayload {
        var zip: Data
        var extensionID: String?
    }

    /// Extract a ZIP payload into a fresh managed directory.
    private static func unpack(zipData zip: Data) throws -> URL {
        let fm = FileManager.default
        let dir = managedDirectory().appendingPathComponent(UUID().uuidString, isDirectory: true)
        try fm.createDirectory(at: dir, withIntermediateDirectories: true)

        let tmpZip = fm.temporaryDirectory
            .appendingPathComponent(UUID().uuidString)
            .appendingPathExtension("zip")
        try zip.write(to: tmpZip)
        defer { try? fm.removeItem(at: tmpZip) }

        do {
            try validateZipEntryNames(tmpZip)
            try runUnzip(tmpZip, into: dir)
            try validateUnpackedContents(in: dir)
        } catch {
            try? fm.removeItem(at: dir)
            throw error
        }
        return dir
    }

    /// Locate the ZIP archive inside CRX bytes and recover the real extension id
    /// from the package header when the CRX format provides one. A raw `.zip` is
    /// passed through unchanged.
    private static func crxPayload(from data: Data) throws -> CRXPayload {
        let b = [UInt8](data)
        // Raw ZIP (local-file or empty-archive signature) — already unpacked form.
        if b.count >= 2, b[0] == 0x50, b[1] == 0x4B {
            return CRXPayload(zip: data, extensionID: nil)
        }
        // CRX magic "Cr24".
        guard b.count >= 16, b[0] == 0x43, b[1] == 0x72, b[2] == 0x32, b[3] == 0x34 else {
            throw ExtensionInstallError.badArchive
        }
        func u32(_ o: Int) -> Int {
            Int(b[o]) | (Int(b[o + 1]) << 8) | (Int(b[o + 2]) << 16) | (Int(b[o + 3]) << 24)
        }
        let version = u32(4)
        let zipStart: Int
        let packageExtensionID: String?
        switch version {
        case 3:  // "Cr24" + version + headerLength + header + zip
            let headerLength = u32(8)
            guard headerLength <= 16 * 1024 * 1024 else {
                throw ExtensionInstallError.badArchive
            }
            let headerStart = 12
            let headerEnd = headerStart + headerLength
            guard headerEnd < b.count else { throw ExtensionInstallError.badArchive }
            packageExtensionID = crx3ExtensionID(fromHeader: Data(b[headerStart..<headerEnd]))
            zipStart = headerEnd
        case 2:  // "Cr24" + version + pubKeyLength + sigLength + pubKey + sig + zip
            let publicKeyLength = u32(8)
            let signatureLength = u32(12)
            let publicKeyStart = 16
            let publicKeyEnd = publicKeyStart + publicKeyLength
            zipStart = publicKeyEnd + signatureLength
            guard publicKeyEnd <= b.count, zipStart < b.count else {
                throw ExtensionInstallError.badArchive
            }
            packageExtensionID = Self.extensionID(fromPublicKey: Data(b[publicKeyStart..<publicKeyEnd]))
        default:
            throw ExtensionInstallError.badArchive
        }
        guard zipStart < b.count else { throw ExtensionInstallError.badArchive }
        return CRXPayload(zip: Data(b[zipStart...]), extensionID: packageExtensionID)
    }

    private static func crx3ExtensionID(fromHeader header: Data) -> String? {
        parseLengthDelimitedField(number: 10000, in: [UInt8](header))
            .flatMap(crx3SignedDataExtensionID)
    }

    private static func crx3SignedDataExtensionID(from signedData: Data) -> String? {
        parseLengthDelimitedField(number: 1, in: [UInt8](signedData))
            .flatMap(extensionID(fromRawCRXID:))
    }

    private static func parseLengthDelimitedField(number targetField: UInt64,
                                                  in bytes: [UInt8]) -> Data? {
        var index = 0
        while index < bytes.count {
            guard let key = readProtoVarint(bytes, index: &index) else { return nil }
            let fieldNumber = key >> 3
            let wireType = key & 0x7
            switch wireType {
            case 0:
                guard readProtoVarint(bytes, index: &index) != nil else { return nil }
            case 1:
                index += 8
            case 2:
                guard let rawLength = readProtoVarint(bytes, index: &index),
                      rawLength <= UInt64(Int.max)
                else { return nil }
                let length = Int(rawLength)
                guard length >= 0, index + length <= bytes.count else { return nil }
                if fieldNumber == targetField {
                    return Data(bytes[index..<index + length])
                }
                index += length
            case 5:
                index += 4
            default:
                return nil
            }
            guard index <= bytes.count else { return nil }
        }
        return nil
    }

    private static func readProtoVarint(_ bytes: [UInt8], index: inout Int) -> UInt64? {
        var shift: UInt64 = 0
        var result: UInt64 = 0
        while index < bytes.count, shift < 64 {
            let byte = bytes[index]
            index += 1
            result |= UInt64(byte & 0x7f) << shift
            if byte & 0x80 == 0 { return result }
            shift += 7
        }
        return nil
    }

    private static func extensionID(fromManifestKey key: String) -> String? {
        let compact = key.filter { !$0.isWhitespace }
        guard let data = Data(base64Encoded: compact) else { return nil }
        return extensionID(fromPublicKey: data)
    }

    private static func extensionID(fromPublicKey publicKey: Data) -> String {
        extensionID(fromRawCRXID: Data(SHA256.hash(data: publicKey).prefix(16))) ?? ""
    }

    private static func extensionID(fromRawCRXID rawID: Data) -> String? {
        guard rawID.count == 16 else { return nil }
        let alphabet = Array("abcdefghijklmnop")
        var id = ""
        id.reserveCapacity(32)
        for byte in rawID {
            id.append(alphabet[Int(byte >> 4)])
            id.append(alphabet[Int(byte & 0x0f)])
        }
        return id
    }

    private static func validateZipEntryNames(_ zip: URL) throws {
        let task = Process()
        task.executableURL = URL(fileURLWithPath: "/usr/bin/zipinfo")
        task.arguments = ["-1", zip.path]
        let outPipe = Pipe()
        let errPipe = Pipe()
        task.standardOutput = outPipe
        task.standardError = errPipe
        do {
            try task.run()
        } catch {
            throw ExtensionInstallError.unzip(error.localizedDescription)
        }
        let outData = outPipe.fileHandleForReading.readDataToEndOfFile()
        let errData = errPipe.fileHandleForReading.readDataToEndOfFile()
        task.waitUntilExit()
        guard task.terminationStatus == 0 else {
            let msg = String(data: errData, encoding: .utf8)?
                .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            throw ExtensionInstallError.unzip(msg.isEmpty ? "zipinfo failed." : msg)
        }
        guard let listing = String(data: outData, encoding: .utf8) else {
            throw ExtensionInstallError.badArchive
        }
        for rawName in listing.split(whereSeparator: \.isNewline) {
            try validateArchiveEntryName(String(rawName))
        }
    }

    private static func validateArchiveEntryName(_ name: String) throws {
        guard !name.isEmpty,
              !name.hasPrefix("/"),
              !name.hasPrefix("\\"),
              !name.contains("\\")
        else {
            throw ExtensionInstallError.unsafeArchivePath(name)
        }
        if name.range(of: #"^[A-Za-z]:"#,
                      options: .regularExpression) != nil {
            throw ExtensionInstallError.unsafeArchivePath(name)
        }

        let parts = name.split(separator: "/", omittingEmptySubsequences: false)
        for (index, part) in parts.enumerated() {
            if part == "." || part == ".." ||
                (part.isEmpty && index != parts.indices.last) {
                throw ExtensionInstallError.unsafeArchivePath(name)
            }
        }
    }

    private static func validateUnpackedContents(in dir: URL) throws {
        let fm = FileManager.default
        let root = dir.standardized.resolvingSymlinksInPath().path
        let prefix = root.hasSuffix("/") ? root : root + "/"
        guard let enumerator = fm.enumerator(
            at: dir,
            includingPropertiesForKeys: [.isSymbolicLinkKey],
            options: [.skipsPackageDescendants]
        ) else { return }

        for case let item as URL in enumerator {
            let resolved = item.standardized.resolvingSymlinksInPath().path
            guard resolved == root || resolved.hasPrefix(prefix) else {
                let relative = item.path.replacingOccurrences(of: dir.path + "/", with: "")
                throw ExtensionInstallError.unsafeArchivePath(relative)
            }
        }
    }

    /// Extract `zip` into `dir` using the system unzip. Exit status 1 is a
    /// non-fatal warning (e.g. extra bytes), which CRX payloads can trip.
    private static func runUnzip(_ zip: URL, into dir: URL) throws {
        let task = Process()
        task.executableURL = URL(fileURLWithPath: "/usr/bin/unzip")
        task.arguments = ["-o", "-q", zip.path, "-d", dir.path]
        let errPipe = Pipe()
        task.standardError = errPipe
        task.standardOutput = Pipe()
        do {
            try task.run()
        } catch {
            throw ExtensionInstallError.unzip(error.localizedDescription)
        }
        let errData = errPipe.fileHandleForReading.readDataToEndOfFile()
        task.waitUntilExit()
        if task.terminationStatus > 1 {
            let msg = String(data: errData, encoding: .utf8)?
                .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            throw ExtensionInstallError.unzip(msg.isEmpty ? "unzip failed." : msg)
        }
    }

    // MARK: - Install feedback

    fileprivate func presentInstalledAlert(_ ext: BrowserExtension) {
        let alert = NSAlert()
        alert.messageText = "Added “\(ext.name)” to Mori"
        alert.informativeText = "Reload pages to run its supported content scripts."
        alert.addButton(withTitle: "OK")
        alert.runModal()
    }

    fileprivate func presentInstallError(_ error: Error) {
        lastError = error.localizedDescription
        let alert = NSAlert()
        alert.alertStyle = .warning
        alert.messageText = "Couldn't install extension"
        alert.informativeText = error.localizedDescription
        alert.addButton(withTitle: "OK")
        alert.runModal()
    }

    // MARK: - Persistence

    private func persist() {
        guard !catalogIsEphemeral else { return }
        if let data = try? JSONEncoder().encode(extensions) {
            defaults.set(data, forKey: Key.catalog)
        }
        let enabled = extensions.filter(\.enabled).map(\.path)
        defaults.set(enabled, forKey: Key.enabledPaths)
    }

    private func persistUninstallURLs() {
        guard !catalogIsEphemeral else { return }
        defaults.set(uninstallURLs, forKey: Key.uninstallURLs)
    }

    // MARK: - Helpers

    /// Reported to the Web Store update service. Matches the embedded Chromium
    /// (CEF 148 → Chrome 148) so it serves a compatible CRX.
    static let chromeProdVersion = "148.0"
    private static let chromeUserAgent =
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 "
        + "(KHTML, like Gecko) Chrome/148.0.0.0 Safari/537.36"

    /// Extract the 32-character extension id from a Chrome Web Store detail URL,
    /// or nil if `urlString` isn't such a page. Web Store ids are 32 letters in
    /// the range a–p (a base-16 encoding), appearing as the last path segment.
    static func webStoreExtensionID(from urlString: String) -> String? {
        guard let url = URL(string: urlString), let host = url.host else { return nil }
        guard host == "chromewebstore.google.com"
            || host == "chrome.google.com" else { return nil }
        let id = url.pathComponents.last { segment in
            segment.count == 32 && segment.allSatisfy { ("a"..."p").contains($0) }
        }
        return id
    }

    private static func chromeExtensionIDCandidate(from name: String) -> String? {
        let base = (name as NSString).deletingPathExtension.lowercased()
        guard base.count == 32,
              base.allSatisfy({ ("a"..."p").contains($0) })
        else { return nil }
        return base
    }

    static func managedDirectory() -> URL {
        let base = FileManager.default.urls(for: .applicationSupportDirectory,
                                            in: .userDomainMask).first
            ?? URL(fileURLWithPath: NSTemporaryDirectory())
        return base
            .appendingPathComponent("MoriBrowser", isDirectory: true)
            .appendingPathComponent("Extensions", isDirectory: true)
    }

    private struct ManifestMeta {
        var name: String
        var version: String
        var detail: String
        var iconPath: String?
        var popupPage: String?
        var optionsPage: String?
        var manifestKeyExtensionID: String?
    }

    /// Parse the manifest of an unpacked extension at `dir`. Resolves the
    /// localized `__MSG_…__` placeholders that real extensions use for name and
    /// description, and picks the highest-resolution declared icon.
    private static func readManifest(at dir: URL) -> ManifestMeta? {
        guard let json = readManifestJSON(at: dir)
        else { return nil }

        let locale = json["default_locale"] as? String
        let messages = locale.flatMap { loadMessages(at: dir, locale: $0) }

        func localized(_ raw: String?) -> String {
            guard let raw else { return "" }
            // "__MSG_keyName__" → the message's "message" value. Chrome matches
            // message keys case-insensitively, so look up on the lowercased key.
            guard raw.hasPrefix("__MSG_"), raw.hasSuffix("__") else { return raw }
            let key = String(raw.dropFirst(6).dropLast(2)).lowercased()
            return messages?[key] ?? raw
        }

        let name = localized(json["name"] as? String)
        let version = json["version"] as? String ?? ""
        let detail = localized(json["description"] as? String)

        return ManifestMeta(
            name: name.isEmpty ? dir.lastPathComponent : name,
            version: version,
            detail: detail,
            iconPath: bestIconPath(json: json, dir: dir),
            popupPage: popupPage(json: json),
            optionsPage: optionsPage(json: json),
            manifestKeyExtensionID: (json["key"] as? String)
                .flatMap(extensionID(fromManifestKey:)))
    }

    private static func readManifestJSON(at dir: URL) -> [String: Any]? {
        let manifestURL = dir.appendingPathComponent("manifest.json")
        guard let data = try? Data(contentsOf: manifestURL),
              let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return nil }
        return json
    }

    /// The action popup page (MV3 `action`, MV2 `browser_action`/`page_action`).
    private static func popupPage(json: [String: Any]) -> String? {
        for key in ["action", "browser_action", "page_action"] {
            if let action = json[key] as? [String: Any],
               let popup = action["default_popup"] as? String, !popup.isEmpty {
                return popup
            }
        }
        return nil
    }

    /// The options page (`options_ui.page` preferred, else legacy `options_page`).
    private static func optionsPage(json: [String: Any]) -> String? {
        if let ui = json["options_ui"] as? [String: Any],
           let page = ui["page"] as? String, !page.isEmpty {
            return page
        }
        if let page = json["options_page"] as? String, !page.isEmpty {
            return page
        }
        return nil
    }

    /// Load `_locales/<locale>/messages.json` as a lowercased-key → message map
    /// (Chrome matches `__MSG_…__` keys case-insensitively).
    private static func loadMessages(at dir: URL, locale: String) -> [String: String]? {
        let url = dir
            .appendingPathComponent("_locales", isDirectory: true)
            .appendingPathComponent(locale, isDirectory: true)
            .appendingPathComponent("messages.json")
        guard let data = try? Data(contentsOf: url),
              let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return nil }
        var out: [String: String] = [:]
        for (key, value) in json {
            if let entry = value as? [String: Any],
               let message = entry["message"] as? String {
                out[key.lowercased()] = message
            }
        }
        return out
    }

    /// Pick the largest icon declared in `icons` (or `action.default_icon`),
    /// returning its absolute path if the file exists.
    private static func bestIconPath(json: [String: Any], dir: URL) -> String? {
        func largest(_ dict: [String: Any]) -> String? {
            dict.compactMap { key, value -> (Int, String)? in
                guard let size = Int(key), let rel = value as? String else { return nil }
                return (size, rel)
            }
            .max { $0.0 < $1.0 }?.1
        }

        var relative: String?
        if let icons = json["icons"] as? [String: Any] {
            relative = largest(icons)
        }
        if relative == nil,
           let action = (json["action"] ?? json["browser_action"]) as? [String: Any] {
            if let icon = action["default_icon"] as? [String: Any] {
                relative = largest(icon)
            } else if let icon = action["default_icon"] as? String {
                relative = icon
            }
        }
        guard let relative else { return nil }
        let path = dir.appendingPathComponent(relative).path
        return FileManager.default.fileExists(atPath: path) ? path : nil
    }
}

// MARK: - Errors

enum ExtensionInstallError: LocalizedError {
    case badArchive
    case noManifest
    case unzip(String)
    case download(String)
    case unsafeArchivePath(String)
    case extensionIDMismatch(expected: String, actual: String)

    var errorDescription: String? {
        switch self {
        case .badArchive:
            return "That file isn't a valid Chrome extension package."
        case .noManifest:
            return "The extension package is missing its manifest.json."
        case .unzip(let detail):
            return "Couldn't unpack the extension. \(detail)"
        case .download(let detail):
            return "Couldn't download the extension. \(detail)"
        case .unsafeArchivePath:
            return "The extension package contains unsafe file paths."
        case .extensionIDMismatch(let expected, let actual):
            return "The extension package id \(actual) didn't match the requested id \(expected)."
        }
    }
}

// MARK: - Native bridge

/// Entry point the native download handler calls when a `.crx` finishes
/// downloading, so the file installs into Mori instead of being handed off to
/// whatever app owns the `.crx` type (typically Google Chrome).
@objc(MoriExtensionBridge)
final class MoriExtensionBridge: NSObject {
    @objc static func installCRX(atPath path: String) {
        installCRX(atPath: path, fallbackURL: "")
    }

    @objc static func installCRX(atPath path: String, fallbackURL urlString: String) {
        let url = URL(fileURLWithPath: path)
        Task { @MainActor in
            do {
                let data: Data
                if FileManager.default.fileExists(atPath: path) {
                    data = try Data(contentsOf: url)
                } else if let fallbackURL = URL(string: urlString), !urlString.isEmpty {
                    let (downloaded, response) = try await URLSession.shared.data(from: fallbackURL)
                    if let http = response as? HTTPURLResponse,
                       !(200...299).contains(http.statusCode) {
                        throw ExtensionInstallError.download("The CRX fallback returned HTTP \(http.statusCode).")
                    }
                    guard !downloaded.isEmpty else {
                        throw ExtensionInstallError.download("The CRX fallback returned an empty response.")
                    }
                    data = downloaded
                } else {
                    data = try Data(contentsOf: url)
                }
                let ext = try ExtensionStore.shared.installCRXData(
                    data, name: url.lastPathComponent)
                NSLog("Mori CRX install complete id=%@ name=%@",
                      ext.id as NSString, ext.name as NSString)
                if ProcessInfo.processInfo.environment["MORI_EXTENSION_SMOKE_RESULT_PATH"] == nil {
                    ExtensionStore.shared.presentInstalledAlert(ext)
                }
            } catch {
                NSLog("Mori CRX install failed path=%@ error=%@",
                      path as NSString, error.localizedDescription as NSString)
                if ProcessInfo.processInfo.environment["MORI_EXTENSION_SMOKE_RESULT_PATH"] == nil {
                    ExtensionStore.shared.presentInstallError(error)
                }
            }
            try? FileManager.default.removeItem(at: url)
        }
    }
}
