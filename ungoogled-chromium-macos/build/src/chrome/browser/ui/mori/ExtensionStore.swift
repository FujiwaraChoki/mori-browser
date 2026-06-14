import SwiftUI
import AppKit

/// One installed Chromium extension, mirrored from Chrome's extension service
/// via `MoriChromeExtensions`. Mori never owns extension state: this is a
/// read-only snapshot, and every mutation goes back through the bridge.
struct ChromeExtensionInfo: Identifiable, Equatable {
    let id: String
    let name: String
    let shortName: String
    let detail: String          // manifest "description"
    let version: String
    let enabled: Bool
    let pinned: Bool            // ToolbarActionsModel pin (own toolbar icon)
    let mayDisable: Bool        // false for policy-installed extensions
    let hasOptionsPage: Bool
    let hasPopup: Bool          // action popup for the active tab
    let badgeText: String
    let actionTitle: String
    let badgeBackgroundColor: [Int]   // RGBA 0-255
    let badgeTextColor: [Int]
    let icon: NSImage?
    let homepageURL: String
    let webStoreURL: String
    let installType: String     // "normal" | "development" | "other"

    init?(dictionary: [String: Any]) {
        guard let id = dictionary["id"] as? String, !id.isEmpty,
              let name = dictionary["name"] as? String
        else { return nil }
        self.id = id
        self.name = name
        shortName = dictionary["shortName"] as? String ?? name
        detail = dictionary["description"] as? String ?? ""
        version = dictionary["version"] as? String ?? ""
        enabled = (dictionary["enabled"] as? NSNumber)?.boolValue ?? false
        pinned = (dictionary["pinned"] as? NSNumber)?.boolValue ?? false
        mayDisable = (dictionary["mayDisable"] as? NSNumber)?.boolValue ?? true
        hasOptionsPage = (dictionary["hasOptionsPage"] as? NSNumber)?.boolValue ?? false
        hasPopup = (dictionary["hasPopup"] as? NSNumber)?.boolValue ?? false
        badgeText = dictionary["badgeText"] as? String ?? ""
        actionTitle = dictionary["actionTitle"] as? String ?? ""
        badgeBackgroundColor = Self.colorComponents(dictionary["badgeBackgroundColor"])
            ?? [217, 48, 37, 255]
        badgeTextColor = Self.colorComponents(dictionary["badgeTextColor"])
            ?? [255, 255, 255, 255]
        icon = dictionary["icon"] as? NSImage
        homepageURL = dictionary["homepageURL"] as? String ?? ""
        webStoreURL = dictionary["webStoreURL"] as? String ?? ""
        installType = dictionary["installType"] as? String ?? "normal"
    }

    private static func colorComponents(_ raw: Any?) -> [Int]? {
        guard let numbers = raw as? [NSNumber], numbers.count >= 3 else { return nil }
        var components = numbers.prefix(4).map { min(max($0.intValue, 0), 255) }
        if components.count == 3 { components.append(255) }
        return components
    }
}

/// A keyboard command declared by an enabled extension, as registered with
/// Chrome's CommandService.
struct ExtensionCommand: Equatable {
    let extensionID: String
    let extensionName: String
    let commandName: String
    let detail: String
    let shortcut: String   // portable "Ctrl+Shift+K" form (Ctrl ⇒ ⌘ on macOS)
    let isAction: Bool     // _execute_action and MV2 equivalents

    init?(dictionary: [String: Any]) {
        guard let extensionID = dictionary["extensionId"] as? String, !extensionID.isEmpty,
              let commandName = dictionary["commandName"] as? String, !commandName.isEmpty,
              let shortcut = dictionary["shortcut"] as? String, !shortcut.isEmpty
        else { return nil }
        self.extensionID = extensionID
        self.commandName = commandName
        self.shortcut = shortcut
        extensionName = dictionary["extensionName"] as? String ?? ""
        detail = dictionary["description"] as? String ?? ""
        isAction = (dictionary["isAction"] as? NSNumber)?.boolValue ?? false
    }
}

/// Observable mirror of Chrome's extension service for Mori's SwiftUI chrome.
/// Chrome owns all extension behavior (content scripts, service workers,
/// chrome.* APIs, popups, side panels, badges); this store only reflects that
/// state into the UI and forwards user intent back through the bridge.
final class ExtensionStore: ObservableObject {
    static let shared = ExtensionStore()

    @Published private(set) var extensions: [ChromeExtensionInfo] = []
    @Published private(set) var commands: [ExtensionCommand] = []
    /// Web Store ids with an install in flight, so the UI can show progress.
    @Published private(set) var installingIDs: Set<String> = []
    /// Surfaced to the UI when an install fails.
    @Published var lastError: String?
    /// The extension whose side panel is showing (hosted by Chrome, rendered
    /// in Mori's side panel chrome), if any.
    @Published private(set) var sidePanelExtensionID: String?
    @Published private(set) var sidePanelTitle: String?

    private var observers: [NSObjectProtocol] = []

    init() {
        let center = NotificationCenter.default
        observers.append(center.addObserver(
            forName: Notification.Name("MoriChromeExtensionsChanged"),
            object: nil, queue: .main
        ) { [weak self] _ in self?.refresh() })
        observers.append(center.addObserver(
            forName: Notification.Name("MoriChromeExtensionSidePanelChanged"),
            object: nil, queue: .main
        ) { [weak self] _ in self?.refreshSidePanel() })
        observers.append(center.addObserver(
            forName: Notification.Name("MoriChromeExtensionInstallFinished"),
            object: nil, queue: .main
        ) { [weak self] note in self?.handleInstallFinished(note) })
        observers.append(center.addObserver(
            forName: NSApplication.didBecomeActiveNotification,
            object: nil, queue: .main
        ) { [weak self] _ in self?.refresh() })
        refresh()
    }

    deinit {
        observers.forEach { NotificationCenter.default.removeObserver($0) }
    }

    /// Re-read the full model from Chrome. Cheap; also called on tab switches
    /// so per-tab badge/title state stays current.
    func refresh() {
        extensions = MoriChromeExtensions.installedExtensions()
            .compactMap { ChromeExtensionInfo(dictionary: $0 as? [String: Any] ?? [:]) }
        commands = MoriChromeExtensions.commands()
            .compactMap { ExtensionCommand(dictionary: $0 as? [String: Any] ?? [:]) }
        refreshSidePanel()
    }

    private func refreshSidePanel() {
        sidePanelExtensionID = MoriChromeExtensions.sidePanelExtensionId()
        sidePanelTitle = MoriChromeExtensions.sidePanelTitle()
    }

    /// Pinned, currently-enabled extensions (own omnibox icons), sorted by name.
    var pinnedExtensions: [ChromeExtensionInfo] {
        extensions.filter { $0.pinned && $0.enabled }
    }

    var enabledExtensions: [ChromeExtensionInfo] {
        extensions.filter(\.enabled)
    }

    // MARK: - User intent (forwarded to Chrome)

    /// Run the extension's toolbar action like clicking it in Chrome.
    /// `anchor` is the button's rect in screen coordinates for popup placement.
    func runAction(_ ext: ChromeExtensionInfo, anchor: NSRect = .zero) {
        MoriChromeExtensions.runAction(id: ext.id, anchor: anchor)
    }

    func setEnabled(_ ext: ChromeExtensionInfo, _ enabled: Bool) {
        MoriChromeExtensions.setExtension(id: ext.id, enabled: enabled)
    }

    func togglePinned(_ ext: ChromeExtensionInfo) {
        MoriChromeExtensions.setExtension(id: ext.id, pinned: !ext.pinned)
    }

    /// Confirm with the user, then uninstall through Chrome.
    func remove(_ ext: ChromeExtensionInfo) {
        if !suppressDialogs {
            let alert = NSAlert()
            alert.messageText = "Remove “\(ext.name)”?"
            alert.informativeText = "The extension and its data will be removed from Mori."
            alert.addButton(withTitle: "Remove")
            alert.addButton(withTitle: "Cancel")
            alert.alertStyle = .warning
            guard alert.runModal() == .alertFirstButtonReturn else { return }
        }
        if !MoriChromeExtensions.uninstall(id: ext.id) {
            lastError = "“\(ext.name)” can't be removed."
        }
    }

    func openOptions(_ ext: ChromeExtensionInfo) {
        _ = MoriChromeExtensions.openOptionsPage(id: ext.id)
    }

    func openManagePage() {
        MoriChromeExtensions.openExtensionsPage()
    }

    func openDetailsPage(_ ext: ChromeExtensionInfo) {
        MoriChromeExtensions.openExtensionsPage(id: ext.id)
    }

    func closeSidePanel() {
        MoriChromeExtensions.closeSidePanel()
    }

    // MARK: - Keyboard commands

    /// The command bound to this key event, if any.
    func command(matching event: NSEvent) -> ExtensionCommand? {
        command(matching: MoriShortcutTrigger(event: event))
    }

    func command(matching trigger: MoriShortcutTrigger) -> ExtensionCommand? {
        commands.first { Self.shortcut($0.shortcut, matches: trigger) }
    }

    /// Fire a matched command the way Chrome's keybinding registry would.
    func activate(_ command: ExtensionCommand) {
        if command.isAction {
            MoriChromeExtensions.runAction(id: command.extensionID, anchor: .zero)
        } else {
            MoriChromeExtensions.dispatchCommand(command.commandName,
                                                 extensionId: command.extensionID)
        }
    }

    /// Match a portable "Ctrl+Shift+K" shortcut against a key event. Chrome's
    /// cross-platform "Ctrl" means Command on macOS; "MacCtrl" is the real
    /// Control key.
    private static func shortcut(_ shortcut: String, matches trigger: MoriShortcutTrigger) -> Bool {
        var required = NSEvent.ModifierFlags()
        var key: String?

        for rawToken in shortcut.split(separator: "+") {
            let token = rawToken.trimmingCharacters(in: .whitespacesAndNewlines)
            switch token.lowercased() {
            case "command", "cmd", "meta", "commandorcontrol", "ctrl", "control":
                required.insert(.command)
            case "macctrl":
                required.insert(.control)
            case "alt", "option":
                required.insert(.option)
            case "shift":
                required.insert(.shift)
            default:
                key = MoriShortcutTrigger.normalizedPrintableKey(token)
            }
        }

        guard let key else { return false }
        return trigger.modifiers == required && trigger.key == key
    }

    // MARK: - Installing

    /// Present a folder picker and load the chosen unpacked extension through
    /// Chrome's developer-mode loader.
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

    func importExtension(fromUnpackedFolder source: URL) {
        guard FileManager.default.fileExists(
            atPath: source.appendingPathComponent("manifest.json").path
        ) else {
            lastError = "That folder has no manifest.json — pick an unpacked extension directory."
            return
        }
        guard MoriChromeExtensions.loadUnpacked(atPath: source.path) else {
            lastError = "Chrome's extension service is not ready yet."
            return
        }
        lastError = nil
    }

    /// Download an extension by its Chrome Web Store id and hand the CRX to
    /// Chrome's installer. (Ungoogled builds can't use the store's own install
    /// button, so Mori fetches the package directly from the update service.)
    /// Idempotent per id while a download is in flight.
    func beginWebStoreInstall(extensionID id: String) {
        guard !installingIDs.contains(id) else { return }
        installingIDs.insert(id)
        Task { @MainActor in
            do {
                let data = try await downloadCRX(extensionID: id)
                try installCRXData(data, expectedID: id, name: id)
                // installingIDs is cleared by handleInstallFinished.
            } catch {
                installingIDs.remove(id)
                presentInstallError(error)
            }
        }
    }

    private func handleInstallFinished(_ note: Notification) {
        installingIDs.removeAll()
        let ok = (note.userInfo?["ok"] as? NSNumber)?.boolValue ?? false
        let message = note.userInfo?["error"] as? String ?? ""
        if ok {
            lastError = nil
            return
        }
        lastError = message.isEmpty ? "Couldn't install the extension." : message
        guard !suppressDialogs else { return }
        let alert = NSAlert()
        alert.alertStyle = .warning
        alert.messageText = "Couldn't install extension"
        alert.informativeText = lastError ?? ""
        alert.addButton(withTitle: "OK")
        alert.runModal()
    }

    /// Stage CRX bytes in the install queue and hand them to Chrome.
    fileprivate func installCRXData(_ data: Data, expectedID: String?, name: String) throws {
        let queue = Self.managedDirectory()
            .appendingPathComponent("ChromeInstallQueue", isDirectory: true)
        try FileManager.default.createDirectory(at: queue, withIntermediateDirectories: true)
        let safeName = name.isEmpty ? UUID().uuidString : name
        let file = queue.appendingPathComponent(safeName).appendingPathExtension("crx")
        try data.write(to: file, options: .atomic)
        guard MoriChromeExtensions.installCRX(atPath: file.path, expectedId: expectedID) else {
            throw ExtensionInstallError.download("Chrome's extension service is not ready yet.")
        }
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

    fileprivate func presentInstallError(_ error: Error) {
        lastError = error.localizedDescription
        guard !suppressDialogs else { return }
        let alert = NSAlert()
        alert.alertStyle = .warning
        alert.messageText = "Couldn't install extension"
        alert.informativeText = error.localizedDescription
        alert.addButton(withTitle: "OK")
        alert.runModal()
    }

    /// Automated runs (smoke tests) suppress modal dialogs.
    private var suppressDialogs: Bool {
        ProcessInfo.processInfo.environment["MORI_EXTENSION_SMOKE_RESULT_PATH"] != nil
    }

    // MARK: - Helpers

    /// Reported to the Web Store update service so it serves a compatible CRX.
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
        return url.pathComponents.last { segment in
            segment.count == 32 && segment.allSatisfy { ("a"..."p").contains($0) }
        }
    }

    static func managedDirectory() -> URL {
        let base = FileManager.default.urls(for: .applicationSupportDirectory,
                                            in: .userDomainMask).first
            ?? URL(fileURLWithPath: NSTemporaryDirectory())
        return base
            .appendingPathComponent("MoriBrowser", isDirectory: true)
            .appendingPathComponent("Extensions", isDirectory: true)
    }
}

// MARK: - Errors

enum ExtensionInstallError: LocalizedError {
    case download(String)

    var errorDescription: String? {
        switch self {
        case .download(let detail):
            return "Couldn't download the extension. \(detail)"
        }
    }
}

// MARK: - Native bridge

/// Entry point the download pipeline calls when a `.crx` finishes downloading,
/// so the file installs into Mori instead of being handed off to whatever app
/// owns the `.crx` type (typically Google Chrome).
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
                try ExtensionStore.shared.installCRXData(
                    data,
                    expectedID: nil,
                    name: url.lastPathComponent)
                NSLog("Mori Chrome CRX install queued path=%@", path as NSString)
            } catch {
                NSLog("Mori CRX install failed path=%@ error=%@",
                      path as NSString, error.localizedDescription as NSString)
                ExtensionStore.shared.presentInstallError(error)
            }
            try? FileManager.default.removeItem(at: url)
        }
    }
}
