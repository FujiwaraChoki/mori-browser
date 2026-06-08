import SwiftUI
import AppKit

/// Top-level browser state: the open tabs, selection, and chrome toggles.
final class BrowserStore: ObservableObject {
    @Published var tabs: [BrowserTab] = []
    @Published var selectedTabID: BrowserTab.ID?
    @Published var sidebarVisible: Bool
    /// True while the sidebar's resize handle is being dragged. The web card
    /// freezes its (expensive, async) CEF resize and shows a smooth cover for
    /// the duration, so live dragging stays smooth instead of flickering.
    @Published var isResizingSidebar: Bool = false
    @Published var aiPanelVisible: Bool = false
    @Published var extensionSidePanelURL: String?
    @Published var extensionSidePanelTitle: String?
    @Published var settingsVisible: Bool = false
    @Published var findBarVisible: Bool = false
    /// The new-tab launcher (command palette) overlay.
    @Published var launcherVisible: Bool = false
    /// Seeds the launcher's search field when it opens (e.g. the current URL when
    /// invoked from the address bar). Empty for a blank ⌘T launcher.
    var launcherPrefill: String = ""
    /// When true the launcher edits the *current* tab in place (address-bar
    /// behavior) instead of opening the destination in a fresh tab.
    var launcherEditsCurrentTab: Bool = false
    /// True while the cursor is at the top edge: the web card slides down to
    /// reveal the titlebar (traffic lights) in the chrome above it.
    @Published var topChromeRevealed: Bool = false
    /// The active find-in-page query, held here so the Find Next / Previous menu
    /// commands can drive the search without the bar being focused.
    @Published var findQuery: String = ""

    // MARK: Spaces / pinned tabs / folders (sidebar organization)

    /// The current space's label and emoji shown atop the sidebar.
    @Published var spaceName: String = "Personal"
    @Published var spaceEmoji: String = "✦"

    /// Tabs surfaced as icon tiles in the pinned grid, in order.
    @Published var pinnedTabIDs: [BrowserTab.ID] = []

    /// Collapsible folders grouping tabs.
    @Published var folders: [TabFolder] = []

    /// Folder row that should enter rename mode as soon as it renders.
    @Published var folderIDPendingRename: TabFolder.ID?

    /// Shared, persisted user preferences.
    let settings = BrowserSettings.shared

    /// Drives the sidebar media player from injected-agent broadcasts.
    let media = MediaController()

    /// Recently closed tabs, most-recent last. Powers Reopen Closed Tab and
    /// `chrome.sessions`.
    private struct ClosedTabSession {
        let sessionID: String
        let url: String
        let title: String
        let closedAt: Date
    }
    private var closedTabSessions: [ClosedTabSession] = []
    private struct ExtensionSidePanelOptions {
        var path: String?
        var enabled: Bool = true
    }
    private struct ExtensionTabGroup {
        let id: Int
        var title: String?
        var color: String = "grey"
        var collapsed: Bool = false
        var windowId: Int = 1
        var shared: Bool = false
    }
    private var sidePanelOptions: [String: ExtensionSidePanelOptions] = [:]
    private var sidePanelOpenOnActionClick: [String: Bool] = [:]
    private var extensionTabGroups: [Int: ExtensionTabGroup] = [:]
    private var extensionTabGroupMembership: [BrowserTab.ID: Int] = [:]
    private var nextExtensionTabGroupID = 1
    private let extensionTabGroupColors: Set<String> = [
        "grey", "blue", "red", "yellow", "green", "pink", "purple", "cyan", "orange"
    ]
    private struct PendingIdentityFlow {
        let extensionID: String
        let requestID: String
        let redirectBase: String
        let redirectOrigin: String
    }
    private var pendingIdentityFlows: [BrowserTab.ID: PendingIdentityFlow] = [:]
    private var notificationObservers: [NSObjectProtocol] = []
    private let sessionFileURL: URL
    private var sessionSaveScheduled = false
    private var isRestoringSession = false

    private struct PersistedTab: Codable {
        var id: UUID
        var url: String
        var title: String
        /// Last known favicon URL, so a restored-but-unopened tab can show its
        /// real icon (via the remote-load path) without realizing a CEF browser.
        var faviconURL: String?
    }

    private struct PersistedSession: Codable {
        var tabs: [PersistedTab]
        var selectedTabID: UUID?
        var pinnedTabIDs: [UUID]
        var folders: [TabFolder]
        var spaceName: String
        var spaceEmoji: String
    }

    /// The homepage, sourced from user settings.
    var homeURL: String { settings.homepageURL }

    init() {
        sessionFileURL = BrowserStore.supportDirectory()
            .appendingPathComponent("session.json")
        sidebarVisible = settings.showSidebarOnLaunch
        let startURL = ProcessInfo.processInfo.environment["MORI_START_URL"]
            .flatMap { $0.isEmpty ? nil : $0 }

        isRestoringSession = true
        if let startURL {
            let first = makeTab(url: startURL, title: "New Tab")
            tabs = [first]
            selectedTabID = first.id
        } else if !restoreSession() {
            let first = makeTab(url: settings.homepageURL, title: "New Tab")
            tabs = [first]
            selectedTabID = first.id
        }
        isRestoringSession = false
        notificationObservers.append(
            NotificationCenter.default.addObserver(
                forName: .moriOpenExtensionUninstallURL,
                object: nil,
                queue: .main
            ) { [weak self] note in
                guard let self,
                      let url = note.userInfo?["url"] as? String,
                      !url.isEmpty
                else { return }
                _ = self.newTab(url: url, select: true)
                NSApp.activate(ignoringOtherApps: true)
                NSApp.mainWindow?.makeKeyAndOrderFront(nil)
            })

        // Let the media controller map an engine broadcast back to its tab.
        media.resolveTab = { [weak self] browserId in
            self?.tabs.first {
                $0.hasRealized && Int($0.browserView.browserIdentifier) == browserId
            }
        }
        installExtensionCommandSmokeIfNeeded()
    }

    private func installExtensionCommandSmokeIfNeeded() {
        let env = ProcessInfo.processInfo.environment
        guard let extensionID = env["MORI_EXTENSION_SMOKE_COMMAND_ID"],
              !extensionID.isEmpty
        else { return }
        let commandName = env["MORI_EXTENSION_SMOKE_COMMAND_NAME"] ?? "_execute_action"
        let extraCommandName = env["MORI_EXTENSION_SMOKE_EXTRA_COMMAND_NAME"]
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) { [weak self] in
            guard let self else { return }
            guard let command = ExtensionStore.shared.commands().first(where: {
                $0.extensionID == extensionID && $0.commandName == commandName
            }) else { return }
            self.activateExtensionCommand(command)
        }
        if let extraCommandName, !extraCommandName.isEmpty {
            DispatchQueue.main.asyncAfter(deadline: .now() + 2.0) { [weak self] in
                guard let self else { return }
                guard let command = ExtensionStore.shared.commands().first(where: {
                    $0.extensionID == extensionID && $0.commandName == extraCommandName
                }) else { return }
                self.activateExtensionCommand(command)
            }
        }
    }

    var selectedTab: BrowserTab? {
        tabs.first { $0.id == selectedTabID }
    }

    // MARK: Session restore

    private static func supportDirectory() -> URL {
        let fm = FileManager.default
        let base = fm.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
            ?? fm.temporaryDirectory
        let dir = base.appendingPathComponent("MoriBrowser", isDirectory: true)
        try? fm.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    @discardableResult
    private func restoreSession() -> Bool {
        guard let data = try? Data(contentsOf: sessionFileURL),
              let decoded = try? JSONDecoder().decode(PersistedSession.self, from: data)
        else { return false }

        let restoredTabs = decoded.tabs
            .filter { !$0.url.isEmpty }
            .map { persisted -> BrowserTab in
                let tab = makeTab(id: persisted.id, url: persisted.url,
                                  title: persisted.title.isEmpty ? "New Tab" : persisted.title)
                // Seed the last-known icon so the tab shows it immediately,
                // before (or without) the browser ever being realized.
                tab.faviconURL = persisted.faviconURL
                return tab
            }
        guard !restoredTabs.isEmpty else { return false }

        let liveIDs = Set(restoredTabs.map(\.id))
        tabs = restoredTabs
        selectedTabID = decoded.selectedTabID.flatMap { id in
            liveIDs.contains(id) ? id : nil
        } ?? restoredTabs.first?.id
        pinnedTabIDs = decoded.pinnedTabIDs.filter { liveIDs.contains($0) }
        folders = decoded.folders.map { folder in
            var copy = folder
            copy.tabIDs = copy.tabIDs.filter { liveIDs.contains($0) }
            return copy
        }
        spaceName = decoded.spaceName.isEmpty ? "Personal" : decoded.spaceName
        spaceEmoji = decoded.spaceEmoji.isEmpty ? "✦" : decoded.spaceEmoji
        return true
    }

    func scheduleSessionSave() {
        guard !isRestoringSession, !sessionSaveScheduled else { return }
        sessionSaveScheduled = true
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            self.sessionSaveScheduled = false
            self.saveSession()
        }
    }

    private func saveSession() {
        guard !tabs.isEmpty else { return }
        let liveIDs = Set(tabs.map(\.id))
        let state = PersistedSession(
            tabs: tabs.map { PersistedTab(id: $0.id, url: $0.urlString, title: $0.title, faviconURL: $0.faviconURL) },
            selectedTabID: selectedTabID,
            pinnedTabIDs: pinnedTabIDs.filter { liveIDs.contains($0) },
            folders: folders.map { folder in
                var copy = folder
                copy.tabIDs = copy.tabIDs.filter { liveIDs.contains($0) }
                return copy
            },
            spaceName: spaceName,
            spaceEmoji: spaceEmoji
        )
        guard let data = try? JSONEncoder().encode(state) else { return }
        try? data.write(to: sessionFileURL, options: .atomic)
    }

    private func makeTab(id: BrowserTab.ID = UUID(), url: String, title: String) -> BrowserTab {
        let tab = BrowserTab(id: id, url: url, title: title)
        tab.onRequestNewTab = { [weak self] url in
            self?.newTab(url: url)
        }
        tab.onExtensionTabUpdated = { [weak self] tab, changeInfo in
            self?.emitExtensionTabUpdated(tab, changeInfo: changeInfo)
            if changeInfo["url"] != nil || changeInfo["title"] != nil || changeInfo["favIconUrl"] != nil {
                self?.scheduleSessionSave()
            }
            if let url = changeInfo["url"] as? String {
                self?.completeIdentityFlowIfNeeded(tab: tab, url: url)
            }
        }
        tab.onExtensionNavigationEvent = { [weak self] eventName, tab, details in
            self?.emitExtensionWebNavigation(eventName, tab: tab, details: details)
            if let url = details["url"] as? String {
                self?.completeIdentityFlowIfNeeded(tab: tab, url: url)
            }
        }
        return tab
    }

    // MARK: Tab management

    @discardableResult
    func newTab(url: String? = nil, select: Bool = true) -> BrowserTab {
        let initialURL = MoriURLRewriter.rewrite(url ?? settings.newTabURL)
        let tab = makeTab(url: initialURL, title: "New Tab")
        tabs.append(tab)
        emitExtensionEvent("tabs.onCreated", args: [extensionTabRecord(tab)])
        if select { selectTab(tab.id) }
        scheduleSessionSave()
        return tab
    }

    func selectTab(_ id: BrowserTab.ID) {
        // Switching to a tab leaves the settings page — it covers the web card,
        // so it would otherwise stay parked over the newly selected tab.
        if settingsVisible { settingsVisible = false }
        let previous = selectedTabID
        selectedTabID = id
        selectedTab?.realize()
        if previous != id { scheduleSessionSave() }
        if previous != id, let tab = selectedTab {
            emitExtensionEvent("tabs.onActivated", args: [[
                "tabId": tab.extensionTabID,
                "windowId": 1
            ]])
            emitExtensionEvent("tabs.onHighlighted", args: [[
                "tabIds": [tab.extensionTabID],
                "windowId": 1
            ]])
        }
    }

    // MARK: New-tab launcher (command palette)

    /// Open the new-tab launcher instead of immediately creating a blank tab, so
    /// the user can search, jump to an open tab, or pick from history first.
    func presentLauncher() {
        launcherPrefill = ""
        launcherEditsCurrentTab = false
        withAnimation(Motion.reveal) { launcherVisible = true }
    }

    /// Address-bar behavior: open the launcher seeded with the current tab's URL
    /// (text pre-selected), navigating the *current* tab on commit rather than
    /// spawning a new one. Re-invoking while it's already up toggles it closed.
    func presentLauncherForCurrentTab() {
        if launcherVisible {
            // Already up (e.g. ⌘L pressed twice, or while a ⌘T launcher is open):
            // collapse it rather than stacking another invocation.
            dismissLauncher()
            return
        }
        launcherPrefill = selectedTab?.displayURL ?? ""
        launcherEditsCurrentTab = true
        withAnimation(Motion.reveal) { launcherVisible = true }
    }

    /// ⌘T behavior: open the launcher, or close it again if it's already up.
    func toggleLauncher() {
        if launcherVisible {
            dismissLauncher()
        } else {
            presentLauncher()
        }
    }

    func dismissLauncher() {
        launcherEditsCurrentTab = false
        withAnimation(Motion.reveal) { launcherVisible = false }
    }

    /// Commit typed launcher text. In address-bar mode this loads into the
    /// current tab; otherwise it opens a fresh tab. A blank commit makes a new
    /// blank tab only in new-tab mode (in edit mode it's a no-op).
    func launcherOpen(_ input: String) {
        let text = input.trimmingCharacters(in: .whitespacesAndNewlines)
        let editing = launcherEditsCurrentTab && selectedTab != nil
        dismissLauncher()
        guard !text.isEmpty else {
            if !editing { newTab() }
            return
        }
        if editing {
            navigate(text)
        } else {
            newTab(url: URLInterpreter.resolve(text, settings: settings), select: true)
        }
    }

    /// Open a chosen destination (history / suggestion). Loads into the current
    /// tab in address-bar mode, or a fresh tab otherwise.
    func launcherOpen(url: String) {
        let editing = launcherEditsCurrentTab && selectedTab != nil
        dismissLauncher()
        if editing {
            navigate(url)
        } else {
            newTab(url: url, select: true)
        }
    }

    /// Jump to an already-open tab from the launcher.
    func launcherSwitch(to id: BrowserTab.ID) {
        dismissLauncher()
        selectTab(id)
    }

    func closeTab(_ id: BrowserTab.ID, allowPinned: Bool = false) {
        guard let idx = tabs.firstIndex(where: { $0.id == id }) else { return }
        // Pinned tabs are permanent: a close gesture (Cmd-W, close button) is
        // ignored. They can only be removed by explicitly unpinning them.
        if pinnedTabIDs.contains(id), !allowPinned { return }
        let tab = tabs[idx]
        let extensionTabID = tab.extensionTabID
        // Remember where it was pointing so Cmd-Shift-T can bring it back.
        let url = tab.urlString
        if url != "about:blank", !url.isEmpty {
            closedTabSessions.append(ClosedTabSession(sessionID: UUID().uuidString,
                                                      url: url,
                                                      title: tab.title,
                                                      closedAt: Date()))
            if closedTabSessions.count > 25 { closedTabSessions.removeFirst() }
            emitExtensionEvent("sessions.onChanged", args: [])
        }
        tab.close()
        // Animate the removal so the sidebar row fades + shrinks and the rows
        // below collapse up into the gap, matching Zen's `animateItemClose`.
        withAnimation(Motion.tabClose) {
            tabs.remove(at: idx)
            pinnedTabIDs.removeAll { $0 == id }
            for folderIndex in folders.indices {
                folders[folderIndex].tabIDs.removeAll { $0 == id }
            }
        }
        removeTabFromExtensionGroup(id, emitTabUpdate: false)
        emitExtensionEvent("tabs.onRemoved", args: [
            extensionTabID,
            ["windowId": 1, "isWindowClosing": false]
        ])

        if tabs.isEmpty {
            // Always keep at least one tab open.
            let fresh = newTab(select: true)
            selectedTabID = fresh.id
            return
        }
        if selectedTabID == id {
            let newIndex = min(idx, tabs.count - 1)
            selectTab(tabs[newIndex].id)
        }
        scheduleSessionSave()
    }

    func moveTab(from source: IndexSet, to destination: Int) {
        let before = tabs
        let movedIDs = Set(source.compactMap { before.indices.contains($0) ? before[$0].id : nil })
        tabs.move(fromOffsets: source, toOffset: destination)
        emitExtensionTabMoveEvents(before: before, movedIDs: movedIDs)
        scheduleSessionSave()
    }

    @discardableResult
    func duplicateTab(_ id: BrowserTab.ID, select: Bool = true) -> BrowserTab? {
        guard let tab = tabs.first(where: { $0.id == id }) else { return nil }
        let duplicate = makeTab(url: tab.urlString, title: tab.title)
        duplicate.faviconURL = tab.faviconURL
        let sourceIndex = tabs.firstIndex { $0.id == id } ?? tabs.count - 1
        tabs.insert(duplicate, at: min(sourceIndex + 1, tabs.count))
        emitExtensionEvent("tabs.onCreated", args: [extensionTabRecord(duplicate)])
        if select { selectTab(duplicate.id) }
        scheduleSessionSave()
        return duplicate
    }

    func copyURL(of id: BrowserTab.ID) {
        guard let url = tabs.first(where: { $0.id == id })?.urlString,
              !url.isEmpty
        else { return }
        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        pasteboard.setString(url, forType: .string)
    }

    /// Copy the active tab's link to the clipboard, confirming with a toast.
    /// Backs the ⌘⇧C shortcut.
    func copyCurrentTabURL() {
        guard let url = selectedTab?.urlString, !url.isEmpty else {
            ToastCenter.shared.show("No link to copy", icon: "xmark", style: .warning)
            return
        }
        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        pasteboard.setString(url, forType: .string)
        ToastCenter.shared.show("Link copied to clipboard", icon: "link", style: .success)
    }

    func closeOtherTabs(than id: BrowserTab.ID) {
        let ids = tabs
            .map(\.id)
            .filter { $0 != id && !pinnedTabIDs.contains($0) }
        for tabID in ids {
            closeTab(tabID)
        }
        selectTab(id)
    }

    func closeTabsToRight(of id: BrowserTab.ID) {
        guard let index = tabs.firstIndex(where: { $0.id == id }),
              index + 1 < tabs.count
        else { return }
        let ids = tabs[(index + 1)...]
            .map(\.id)
            .filter { !pinnedTabIDs.contains($0) }
        for tabID in ids {
            closeTab(tabID)
        }
        selectTab(id)
    }

    func hasClosableTabsToRight(of id: BrowserTab.ID) -> Bool {
        guard let index = tabs.firstIndex(where: { $0.id == id }),
              index + 1 < tabs.count
        else { return false }
        return tabs[(index + 1)...].contains { !pinnedTabIDs.contains($0.id) }
    }

    /// Reopen the most recently closed tab, restoring its last URL.
    func reopenClosedTab() {
        guard let session = closedTabSessions.popLast() else { return }
        _ = newTab(url: session.url, select: true)
        emitExtensionEvent("sessions.onChanged", args: [])
    }

    /// Select the tab at a 1-based slot (Cmd-1…Cmd-9). By convention the
    /// highest slot, 9, always jumps to the *last* tab regardless of count.
    ///
    /// Slots follow the sidebar's visual order — pinned tabs first — so Cmd-1
    /// lands on the first pinned tab. Recomputed on each press, so it tracks
    /// pinning/unpinning dynamically.
    func selectTab(atOrdinal ordinal: Int) {
        let ordered = orderedTabsForShortcuts
        guard !ordered.isEmpty else { return }
        let index = ordinal >= 9 ? ordered.count - 1 : ordinal - 1
        guard ordered.indices.contains(index) else { return }
        selectTab(ordered[index].id)
    }

    /// Tab order used by the Cmd-1…Cmd-9 shortcuts: pinned tabs first (matching
    /// the sidebar), then the remaining tabs in their existing order.
    private var orderedTabsForShortcuts: [BrowserTab] {
        pinnedTabs + tabs.filter { !pinnedTabIDs.contains($0.id) }
    }

    /// Cycle to the next/previous tab, wrapping around at the ends.
    func selectNextTab() { cycleTab(by: 1) }
    func selectPreviousTab() { cycleTab(by: -1) }

    private func cycleTab(by delta: Int) {
        guard tabs.count > 1,
              let current = tabs.firstIndex(where: { $0.id == selectedTabID })
        else { return }
        let next = (current + delta + tabs.count) % tabs.count
        selectTab(tabs[next].id)
    }

    // MARK: Navigation on the active tab

    func goBack() { selectedTab?.goBack() }
    func goForward() { selectedTab?.goForward() }
    func reload() { selectedTab?.reload() }
    func reloadIgnoringCache() { selectedTab?.reloadIgnoringCache() }
    func stop() { selectedTab?.stop() }

    func zoomIn() { selectedTab?.zoomIn() }
    func zoomOut() { selectedTab?.zoomOut() }
    func resetZoom() { selectedTab?.resetZoom() }

    func toggleDevTools() { selectedTab?.toggleDevTools() }
    func printPage() { selectedTab?.printPage() }

    // MARK: Find-in-page

    func showFindBar() {
        withAnimation(Motion.snappy) { findBarVisible = true }
        if !findQuery.isEmpty { selectedTab?.find(findQuery, forward: true) }
    }

    func hideFindBar() {
        selectedTab?.stopFind()
        withAnimation(Motion.snappy) { findBarVisible = false }
    }

    func toggleFindBar() {
        if findBarVisible { hideFindBar() } else { showFindBar() }
    }

    /// Re-run the current query, advancing to the next/previous match. Opens the
    /// bar first if it's closed (Cmd-G with no bar yet).
    func findNext(forward: Bool) {
        guard !findQuery.isEmpty else { showFindBar(); return }
        if !findBarVisible { showFindBar() }
        selectedTab?.find(findQuery, forward: forward)
    }

    /// Interpret omnibox text as either a URL or a search query.
    func navigate(_ input: String) {
        // Loading a URL into the current tab means the user wants the page, not
        // the settings page that's covering it.
        if settingsVisible { settingsVisible = false }
        let resolved = MoriURLRewriter.rewrite(
            URLInterpreter.resolve(input, settings: settings))
        selectedTab?.load(resolved)
    }

    // MARK: Extension tab bridge

    func handleExtensionRuntime(method: String, args: NSDictionary) -> NSDictionary {
        switch method {
        case "runtime.openOptionsPage":
            guard let extensionID = args["extensionId"] as? String, !extensionID.isEmpty else {
                return ["error": "Missing extension id."]
            }
            guard let url = ExtensionStore.shared.optionsURL(forExtensionID: extensionID) else {
                return ["error": "Extension has no options page."]
            }
            _ = newTab(url: url, select: true)
            NSApp.activate(ignoringOtherApps: true)
            NSApp.mainWindow?.makeKeyAndOrderFront(nil)
            return ["result": NSNull()]

        case "runtime.getContexts":
            guard let extensionID = args["extensionId"] as? String, !extensionID.isEmpty else {
                return ["error": "Missing extension id."]
            }
            let filter = args["filter"] as? NSDictionary ?? [:]
            return ["result": ExtensionStore.shared.runtimeContexts(forExtensionID: extensionID, filter: filter)]

        case "runtime.setUninstallURL":
            guard let extensionID = args["extensionId"] as? String, !extensionID.isEmpty else {
                return ["error": "Missing extension id."]
            }
            let url = args["url"] as? String ?? ""
            return ExtensionStore.shared.setUninstallURL(forExtensionID: extensionID, url: url)

        case "runtime.reload":
            guard let extensionID = args["extensionId"] as? String, !extensionID.isEmpty else {
                return ["error": "Missing extension id."]
            }
            NotificationCenter.default.post(
                name: .moriReloadExtensionRuntime,
                object: nil,
                userInfo: ["extensionId": extensionID]
            )
            if let sidePanelURL = extensionSidePanelURL,
               sidePanelURL.lowercased().hasPrefix("mori-extension://\(extensionID.lowercased())/") {
                extensionSidePanelURL = nil
                extensionSidePanelTitle = nil
            }
            return ["result": NSNull()]

        case "identity.launchWebAuthFlow":
            guard let extensionID = args["extensionId"] as? String, !extensionID.isEmpty,
                  let requestID = args["requestId"] as? String, !requestID.isEmpty
            else {
                return ["error": "Missing identity flow metadata."]
            }
            let details = args["details"] as? NSDictionary ?? [:]
            guard (details["interactive"] as? NSNumber)?.boolValue ?? true else {
                return ["error": "Non-interactive identity flows are not supported in Mori yet."]
            }
            guard let url = details["url"] as? String, !url.isEmpty else {
                return ["error": "Missing identity URL."]
            }
            let redirectOrigin = "https://\(extensionID).chromiumapp.org/"
            let tab = newTab(url: url, select: true)
            pendingIdentityFlows[tab.id] = PendingIdentityFlow(
                extensionID: extensionID,
                requestID: requestID,
                redirectBase: redirectOrigin,
                redirectOrigin: redirectOrigin
            )
            completeIdentityFlowIfNeeded(tab: tab, url: tab.urlString)
            return ["deferred": true, "result": NSNull()]

        case "sidePanel.setOptions":
            guard let extensionID = args["extensionId"] as? String, !extensionID.isEmpty else {
                return ["error": "Missing extension id."]
            }
            let details = args["details"] as? NSDictionary ?? [:]
            var options = sidePanelOptions[extensionID] ?? ExtensionSidePanelOptions()
            if let path = details["path"] as? String {
                options.path = path.isEmpty ? nil : path
            }
            if let enabled = details["enabled"] as? NSNumber {
                options.enabled = enabled.boolValue
            }
            sidePanelOptions[extensionID] = options
            if !options.enabled, extensionSidePanelURL == sidePanelURL(forExtensionID: extensionID) {
                closeExtensionSidePanel()
            }
            return ["result": NSNull()]

        case "sidePanel.getOptions":
            guard let extensionID = args["extensionId"] as? String, !extensionID.isEmpty else {
                return ["error": "Missing extension id."]
            }
            let options = sidePanelOptions[extensionID] ?? ExtensionSidePanelOptions()
            var result: [String: Any] = ["enabled": options.enabled]
            if let path = options.path {
                result["path"] = path
            } else if let url = ExtensionStore.shared.sidePanelURL(forExtensionID: extensionID),
                      let parsed = URL(string: url) {
                result["path"] = String(parsed.path.drop(while: { $0 == "/" }))
            }
            return ["result": result]

        case "sidePanel.open":
            guard let extensionID = args["extensionId"] as? String, !extensionID.isEmpty else {
                return ["error": "Missing extension id."]
            }
            guard openExtensionSidePanel(extensionID: extensionID) else {
                return ["error": "This extension does not have an enabled side panel."]
            }
            return ["result": NSNull()]

        case "sidePanel.close":
            closeExtensionSidePanel()
            return ["result": NSNull()]

        case "sidePanel.setPanelBehavior":
            guard let extensionID = args["extensionId"] as? String, !extensionID.isEmpty else {
                return ["error": "Missing extension id."]
            }
            let behavior = args["behavior"] as? NSDictionary ?? [:]
            if let value = behavior["openPanelOnActionClick"] as? NSNumber {
                sidePanelOpenOnActionClick[extensionID] = value.boolValue
            }
            return ["result": NSNull()]

        case "sidePanel.getPanelBehavior":
            guard let extensionID = args["extensionId"] as? String, !extensionID.isEmpty else {
                return ["error": "Missing extension id."]
            }
            return ["result": [
                "openPanelOnActionClick": sidePanelOpenOnActionClick[extensionID] ?? false
            ]]

        default:
            return ["error": "Unsupported runtime method: \(method)"]
        }
    }

    func handleExtensionSearch(method: String, args: NSDictionary) -> NSDictionary {
        guard method == "search.query" else {
            return ["error": "Unsupported search method: \(method)"]
        }
        let queryInfo = args["queryInfo"] as? NSDictionary ?? [:]
        guard let text = queryInfo["text"] as? String,
              !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        else {
            return ["error": "search.query requires a non-empty text value."]
        }
        let searchURL = settings.searchURL(for: text)
        let hasTabID = queryInfo["tabId"] is NSNumber
        let hasDisposition = queryInfo["disposition"] is String
        if hasTabID && hasDisposition {
            return ["error": "search.query cannot use tabId with disposition."]
        }

        if hasTabID {
            guard let tab = extensionTab(for: queryInfo["tabId"]) else {
                return ["error": "No tab with that id."]
            }
            tab.load(searchURL)
            scheduleSessionSave()
            return ["result": NSNull()]
        }

        let disposition = (queryInfo["disposition"] as? String) ?? "CURRENT_TAB"
        switch disposition {
        case "CURRENT_TAB":
            if let tab = selectedTab {
                tab.load(searchURL)
            } else {
                _ = newTab(url: searchURL, select: true)
            }
        case "NEW_TAB", "NEW_WINDOW":
            _ = newTab(url: searchURL, select: true)
        default:
            return ["error": "Unsupported search disposition: \(disposition)"]
        }
        scheduleSessionSave()
        return ["result": NSNull()]
    }

    func handleExtensionTabs(method: String, args: NSDictionary) -> NSDictionary {
        switch method {
        case "tabs.query":
            let queryInfo = args["queryInfo"] as? NSDictionary ?? [:]
            let result = extensionTabs(matching: queryInfo).map(extensionTabRecord)
            return ["result": result]

        case "tabs.get":
            guard let tab = extensionTab(for: args["tabId"]) else {
                return ["error": "No tab with that id."]
            }
            return ["result": extensionTabRecord(tab)]

        case "tabs.getCurrent":
            guard let tab = selectedTab else {
                return ["result": NSNull()]
            }
            return ["result": extensionTabRecord(tab)]

        case "tabs.create":
            let props = args["createProperties"] as? NSDictionary ?? [:]
            let url = props["url"] as? String
            let active = (props["active"] as? NSNumber)?.boolValue ?? true
            let tab = newTab(url: url, select: active)
            return ["result": extensionTabRecord(tab)]

        case "tabs.duplicate":
            guard let tab = extensionTab(for: args["tabId"]) else {
                return ["error": "No tab with that id."]
            }
            guard let duplicate = duplicateTab(tab.id, select: true) else {
                return ["error": "Could not duplicate that tab."]
            }
            return ["result": extensionTabRecord(duplicate)]

        case "tabs.reload":
            let tab = extensionTab(for: args["tabId"]) ?? selectedTab
            guard let tab else { return ["error": "No active tab."] }
            let props = args["reloadProperties"] as? NSDictionary ?? [:]
            if (props["bypassCache"] as? NSNumber)?.boolValue == true {
                tab.reloadIgnoringCache()
            } else {
                tab.reload()
            }
            return ["result": NSNull()]

        case "tabs.goBack":
            let tab = extensionTab(for: args["tabId"]) ?? selectedTab
            guard let tab else { return ["error": "No active tab."] }
            if tab.canGoBack { tab.goBack() }
            return ["result": NSNull()]

        case "tabs.goForward":
            let tab = extensionTab(for: args["tabId"]) ?? selectedTab
            guard let tab else { return ["error": "No active tab."] }
            if tab.canGoForward { tab.goForward() }
            return ["result": NSNull()]

        case "tabs.getZoom":
            let tab = extensionTab(for: args["tabId"]) ?? selectedTab
            guard let tab else { return ["error": "No active tab."] }
            return ["result": Double(tab.zoomPercent) / 100.0]

        case "tabs.setZoom":
            let tab = extensionTab(for: args["tabId"]) ?? selectedTab
            guard let tab else { return ["error": "No active tab."] }
            let factor = (args["zoomFactor"] as? NSNumber)?.doubleValue ?? 1.0
            tab.setZoomFactor(factor)
            return ["result": NSNull()]

        case "tabs.getZoomSettings":
            return ["result": [
                "mode": "automatic",
                "scope": "per-tab",
                "defaultZoomFactor": 1.0
            ]]

        case "tabs.setZoomSettings":
            return ["result": NSNull()]

        case "tabs.captureVisibleTab":
            guard let tab = selectedTab else {
                return ["error": "No active tab."]
            }
            guard let extensionID = args["extensionId"] as? String, !extensionID.isEmpty,
                  let requestID = args["requestId"] as? String, !requestID.isEmpty else {
                return ["error": "tabs.captureVisibleTab requires an extension request."]
            }
            guard tab.captureVisiblePNGDataURL(extensionID: extensionID, requestID: requestID) else {
                return ["error": "The active tab is not ready to capture."]
            }
            return ["deferred": true, "result": NSNull()]

        case "tabs.remove":
            let tabIDs = extensionTabIDs(from: args["tabIds"])
            guard !tabIDs.isEmpty else {
                return ["error": "tabs.remove requires a tab id."]
            }
            for tabID in tabIDs {
                guard let tab = extensionTab(for: NSNumber(value: tabID)) else {
                    return ["error": "No tab with id \(tabID)."]
                }
                closeTab(tab.id, allowPinned: true)
            }
            return ["result": NSNull()]

        case "tabs.group":
            let options = args["options"] as? NSDictionary ?? [:]
            let tabIDs = extensionTabIDs(from: options["tabIds"])
            guard !tabIDs.isEmpty else {
                return ["error": "tabs.group requires at least one tab id."]
            }
            let groupedTabs = tabIDs.compactMap {
                extensionTab(for: NSNumber(value: $0))
            }
            guard groupedTabs.count == tabIDs.count else {
                return ["error": "No tab with one of those ids."]
            }

            let groupID: Int
            if let existingID = options["groupId"] as? NSNumber {
                groupID = existingID.intValue
                guard extensionTabGroups[groupID] != nil else {
                    return ["error": "No tab group with id \(groupID)."]
                }
            } else {
                let createProperties = options["createProperties"] as? NSDictionary ?? [:]
                groupID = nextExtensionTabGroupID
                nextExtensionTabGroupID += 1
                var group = ExtensionTabGroup(id: groupID)
                if let windowID = createProperties["windowId"] as? NSNumber,
                   windowID.intValue >= 0 {
                    group.windowId = windowID.intValue
                }
                extensionTabGroups[groupID] = group
                emitExtensionEvent("tabGroups.onCreated", args: [extensionTabGroupRecord(group)])
            }

            for tab in groupedTabs {
                if let previousGroupID = extensionTabGroupMembership[tab.id],
                   previousGroupID != groupID {
                    extensionTabGroupMembership.removeValue(forKey: tab.id)
                    pruneEmptyExtensionTabGroup(previousGroupID)
                }
                extensionTabGroupMembership[tab.id] = groupID
                emitExtensionTabUpdated(tab, changeInfo: ["groupId": groupID])
            }
            return ["result": groupID]

        case "tabs.ungroup":
            let tabIDs = extensionTabIDs(from: args["tabIds"])
            guard !tabIDs.isEmpty else {
                return ["error": "tabs.ungroup requires at least one tab id."]
            }
            let ungroupedTabs = tabIDs.compactMap {
                extensionTab(for: NSNumber(value: $0))
            }
            guard ungroupedTabs.count == tabIDs.count else {
                return ["error": "No tab with one of those ids."]
            }
            for tab in ungroupedTabs {
                removeTabFromExtensionGroup(tab.id)
            }
            return ["result": NSNull()]

        case "tabs.move":
            let tabIDs = extensionTabIDs(from: args["tabIds"])
            guard !tabIDs.isEmpty else {
                return ["error": "tabs.move requires at least one tab id."]
            }
            let before = tabs
            let movingTabs = tabIDs.compactMap {
                extensionTab(for: NSNumber(value: $0))
            }
            guard movingTabs.count == tabIDs.count else {
                return ["error": "No tab with one of those ids."]
            }
            let props = args["moveProperties"] as? NSDictionary ?? [:]
            let requestedIndex = (props["index"] as? NSNumber)?.intValue ?? tabs.count
            let movingIDs = Set(movingTabs.map(\.id))
            tabs.removeAll { movingIDs.contains($0.id) }
            var insertionIndex = requestedIndex < 0
                ? tabs.count
                : max(0, min(Int(requestedIndex), tabs.count))
            for tab in movingTabs {
                tabs.insert(tab, at: insertionIndex)
                insertionIndex += 1
            }
            emitExtensionTabMoveEvents(before: before, movedIDs: movingIDs)
            let records = movingTabs.map(extensionTabRecord)
            return ["result": tabIDs.count == 1 ? records[0] : records]

        case "tabs.update":
            let tab = extensionTab(for: args["tabId"]) ?? selectedTab
            guard let tab else { return ["error": "No active tab."] }
            let props = args["updateProperties"] as? NSDictionary ?? [:]
            if let active = props["active"] as? NSNumber, active.boolValue {
                selectTab(tab.id)
            }
            if let url = props["url"] as? String, !url.isEmpty {
                tab.load(MoriURLRewriter.rewrite(url))
            }
            return ["result": extensionTabRecord(tab)]

        case "tabs.highlight":
            let info = args["highlightInfo"] as? NSDictionary ?? [:]
            let indices = extensionTabIndices(from: info["tabs"])
            guard let index = indices.first,
                  tabs.indices.contains(index)
            else { return ["error": "No tab with that index."] }
            selectTab(tabs[index].id)
            return ["result": extensionWindowRecord(populate: true)]

        case "tabs.sendMessage":
            guard let tab = extensionTab(for: args["tabId"]) else {
                return ["error": "No tab with that id."]
            }
            guard let extensionID = args["extensionId"] as? String, !extensionID.isEmpty else {
                return ["error": "Missing extension id."]
            }
            let message = args["message"] ?? NSNull()
            let requestID = args["messageRequestId"] ?? NSNull()
            let sourceURL = args["sourceUrl"] ?? NSNull()
            let sourceOrigin = args["sourceOrigin"] ?? NSNull()
            let options = args["options"] as? NSDictionary ?? [:]
            let frameID = (options["frameId"] as? NSNumber)?.intValue
            let documentID = options["documentId"] as? String
            // The trailing true opts into content-script delivery: tabs.sendMessage
            // is explicitly aimed at the content scripts running in this tab, unlike
            // a runtime.sendMessage broadcast which reaches extension contexts only.
            let dispatch = "window.__moriExtDispatchMessage(\(jsonLiteral(extensionID)),\(jsonLiteral(message)),\(jsonLiteral(requestID)),\(jsonLiteral(sourceURL)),\(jsonLiteral(sourceOrigin)),true);"
            let frameCheck = frameID.map { "__moriFrame===\($0)" }
            let documentCheck = documentID.map { "__moriDocument===\(jsonLiteral($0))" }
            let checks = [frameCheck, documentCheck].compactMap { $0 }
            let source: String
            if checks.isEmpty {
                source = "if(window.__moriExtDispatchMessage){\(dispatch)}"
            } else {
                source = "if(window.__moriExtDispatchMessage){var __moriFrame=Number(globalThis.__moriNativeFrameId);if(!Number.isFinite(__moriFrame)){__moriFrame=globalThis.__moriRuntimeContext&&Number(globalThis.__moriRuntimeContext.frameId);}var __moriDocument=String(globalThis.__moriNativeDocumentId||(globalThis.__moriRuntimeContext&&globalThis.__moriRuntimeContext.documentId)||'');if(\(checks.joined(separator: "&&"))){\(dispatch)}}"
            }
            tab.realize().executeExtensionJavaScript(source, allFrames: true)
            return ["result": NSNull()]

        default:
            return ["error": "Unsupported tabs method: \(method)"]
        }
    }

    func handleExtensionTabGroups(method: String, args: NSDictionary) -> NSDictionary {
        switch method {
        case "tabGroups.get":
            guard let groupID = (args["groupId"] as? NSNumber)?.intValue,
                  let group = extensionTabGroups[groupID]
            else { return ["error": "No tab group with that id."] }
            return ["result": extensionTabGroupRecord(group)]

        case "tabGroups.query":
            let queryInfo = args["queryInfo"] as? NSDictionary ?? [:]
            let result = extensionTabGroups.values
                .filter { extensionTabGroup($0, matches: queryInfo) }
                .sorted { $0.id < $1.id }
                .map(extensionTabGroupRecord)
            return ["result": result]

        case "tabGroups.update":
            guard let groupID = (args["groupId"] as? NSNumber)?.intValue,
                  var group = extensionTabGroups[groupID]
            else { return ["error": "No tab group with that id."] }
            let updateProperties = args["updateProperties"] as? NSDictionary ?? [:]
            if let collapsed = updateProperties["collapsed"] as? NSNumber {
                group.collapsed = collapsed.boolValue
            }
            if let color = updateProperties["color"] as? String,
               extensionTabGroupColors.contains(color) {
                group.color = color
            }
            if updateProperties.allKeys.contains(where: { ($0 as? String) == "title" }) {
                group.title = updateProperties["title"] as? String
            }
            extensionTabGroups[groupID] = group
            let record = extensionTabGroupRecord(group)
            emitExtensionEvent("tabGroups.onUpdated", args: [record])
            return ["result": record]

        case "tabGroups.move":
            guard let groupID = (args["groupId"] as? NSNumber)?.intValue,
                  let group = extensionTabGroups[groupID]
            else { return ["error": "No tab group with that id."] }
            let moveProperties = args["moveProperties"] as? NSDictionary ?? [:]
            let requestedIndex = (moveProperties["index"] as? NSNumber)?.intValue ?? -1
            let groupedIDs = Set(extensionTabGroupMembership.compactMap { entry -> BrowserTab.ID? in
                entry.value == groupID ? entry.key : nil
            })
            guard !groupedIDs.isEmpty else {
                pruneEmptyExtensionTabGroup(groupID)
                return ["error": "No tabs remain in that tab group."]
            }
            let before = tabs
            let groupedTabs = tabs.filter { groupedIDs.contains($0.id) }
            tabs.removeAll { groupedIDs.contains($0.id) }
            let insertionIndex = requestedIndex < 0
                ? tabs.count
                : max(0, min(Int(requestedIndex), tabs.count))
            tabs.insert(contentsOf: groupedTabs, at: insertionIndex)
            emitExtensionTabMoveEvents(before: before, movedIDs: groupedIDs)
            emitExtensionEvent("tabGroups.onMoved", args: [extensionTabGroupRecord(group)])
            return ["result": extensionTabGroupRecord(group)]

        default:
            return ["error": "Unsupported tabGroups method: \(method)"]
        }
    }

    func handleExtensionWindows(method: String, args: NSDictionary) -> NSDictionary {
        switch method {
        case "windows.getCurrent", "windows.getLastFocused":
            let populate = (args["populate"] as? NSNumber)?.boolValue ?? false
            return ["result": extensionWindowRecord(populate: populate)]

        case "windows.get":
            let windowID = (args["windowId"] as? NSNumber)?.intValue ?? 1
            guard windowID == 1 || windowID == -2 else {
                return ["error": "No Mori window with that id."]
            }
            let getInfo = args["getInfo"] as? NSDictionary ?? [:]
            let populate = (getInfo["populate"] as? NSNumber)?.boolValue ?? false
            return ["result": extensionWindowRecord(populate: populate)]

        case "windows.getAll":
            let getInfo = args["getInfo"] as? NSDictionary ?? [:]
            let populate = (getInfo["populate"] as? NSNumber)?.boolValue ?? false
            return ["result": [extensionWindowRecord(populate: populate)]]

        case "windows.create":
            let props = args["createData"] as? NSDictionary ?? [:]
            let urls = extensionWindowCreateURLs(props["url"])
            if urls.isEmpty {
                _ = newTab(select: true)
            } else {
                for (idx, url) in urls.enumerated() {
                    _ = newTab(url: url, select: idx == 0)
                }
            }
            if (props["focused"] as? NSNumber)?.boolValue ?? true {
                NSApp.activate(ignoringOtherApps: true)
                NSApp.mainWindow?.makeKeyAndOrderFront(nil)
            }
            emitExtensionEvent("windows.onCreated",
                               args: [extensionWindowRecord(
                                populate: true,
                                focusedOverride: (props["focused"] as? NSNumber)?.boolValue ?? true
                               )])
            return ["result": extensionWindowRecord(
                populate: true,
                focusedOverride: (props["focused"] as? NSNumber)?.boolValue ?? true
            )]

        case "windows.update":
            let updateInfo = args["updateInfo"] as? NSDictionary ?? [:]
            let window = extensionHostWindow()
            if (updateInfo["focused"] as? NSNumber)?.boolValue == true {
                NSApp.activate(ignoringOtherApps: true)
                window?.makeKeyAndOrderFront(nil)
                emitExtensionEvent("windows.onFocusChanged", args: [1])
            }
            if let state = updateInfo["state"] as? String {
                if state == "minimized" {
                    window?.miniaturize(nil)
                } else if window?.isMiniaturized == true {
                    window?.deminiaturize(nil)
                }
            }
            return ["result": extensionWindowRecord(
                populate: true,
                focusedOverride: (updateInfo["focused"] as? NSNumber)?.boolValue
            )]

        case "windows.remove":
            // Mori currently owns one persistent browser window. A close
            // request from an extension is acknowledged without terminating the
            // app or destroying the user's only window.
            return ["result": NSNull()]

        default:
            return ["error": "Unsupported windows method: \(method)"]
        }
    }

    func handleExtensionDownloads(method: String, args: NSDictionary) -> NSDictionary {
        if method == "downloads.download" {
            let options = args["options"] as? NSDictionary ?? [:]
            guard let url = options["url"] as? String, !url.isEmpty else {
                return ["error": "downloads.download requires a url."]
            }
            guard let extensionID = args["extensionId"] as? String, !extensionID.isEmpty,
                  let requestID = args["requestId"] as? String, !requestID.isEmpty else {
                return ["error": "Missing extension download request metadata."]
            }
            guard let tab = selectedTab ?? tabs.first else {
                return ["error": "No browser tab can start the download."]
            }
            let filename = options["filename"] as? String
            guard tab.startDownload(url: url,
                                    extensionID: extensionID,
                                    requestID: requestID,
                                    filename: filename) else {
                return ["error": "Chromium browser is not ready to start the download."]
            }
            return ["deferred": true, "result": NSNull()]
        }
        return DownloadStore.shared.handleExtensionDownloads(method: method, args: args)
    }

    func handleExtensionSessions(method: String, args: NSDictionary) -> NSDictionary {
        switch method {
        case "sessions.getRecentlyClosed":
            let filter = args["filter"] as? NSDictionary ?? [:]
            let maxResults = max(0, min(
                (filter["maxResults"] as? NSNumber)?.intValue ?? 25,
                25
            ))
            let result = closedTabSessions
                .reversed()
                .prefix(maxResults)
                .map(extensionSessionRecord)
            return ["result": Array(result)]

        case "sessions.getDevices":
            return ["result": []]

        case "sessions.restore":
            let rawSessionID = args["sessionId"] as? String
            let index: Int?
            if let rawSessionID, !rawSessionID.isEmpty {
                index = closedTabSessions.firstIndex { $0.sessionID == rawSessionID }
            } else {
                index = closedTabSessions.indices.last
            }
            guard let index else {
                return ["error": "No recently closed tab to restore."]
            }
            let session = closedTabSessions.remove(at: index)
            let tab = newTab(url: session.url, select: true)
            emitExtensionEvent("sessions.onChanged", args: [])
            return ["result": ["tab": extensionTabRecord(tab)]]

        default:
            return ["error": "Unsupported sessions method: \(method)"]
        }
    }

    private func fallbackExtensionFrameRecord(tab: BrowserTab) -> [String: Any] {
        [
            "errorOccurred": false,
            "tabId": tab.extensionTabID,
            "frameId": 0,
            "parentFrameId": -1,
            "documentId": "mori-\(tab.extensionTabID)-main-document",
            "documentLifecycle": "active",
            "frameType": "outermost_frame",
            "url": tab.urlString
        ]
    }

    func handleExtensionWebNavigation(method: String, args: NSDictionary) -> NSDictionary {
        let details = args["details"] as? NSDictionary ?? [:]
        guard let tab = extensionTab(for: details["tabId"]) else {
            return ["error": "No tab with that id."]
        }

        let records = tab.hasRealized
            ? tab.realize().extensionFrameRecords(withTabID: tab.extensionTabID)
            : []
        let frameRecords = records.isEmpty ? [fallbackExtensionFrameRecord(tab: tab)] : records

        switch method {
        case "webNavigation.getFrame":
            let frameID = (details["frameId"] as? NSNumber)?.intValue ?? 0
            let documentID = details["documentId"] as? String
            let result = frameRecords.first { record in
                if let documentID {
                    return (record["documentId"] as? String) == documentID
                }
                return (record["frameId"] as? NSNumber)?.intValue == frameID
            }
            return ["result": result ?? NSNull()]

        case "webNavigation.getAllFrames":
            return ["result": frameRecords]

        default:
            return ["error": "Unsupported webNavigation method: \(method)"]
        }
    }

    func handleExtensionScripting(method: String, args: NSDictionary) -> NSDictionary {
        let target = args["target"] as? NSDictionary ?? [:]
        let tab = extensionTab(for: target["tabId"]) ?? selectedTab
        guard let tab else { return ["error": "No target tab."] }

        let frameIds = target["frameIds"] as? [NSNumber] ?? []
        let documentIds = target["documentIds"] as? [String] ?? []
        let allFrames = ((target["allFrames"] as? NSNumber)?.boolValue ?? false) || !frameIds.isEmpty || !documentIds.isEmpty
        switch method {
        case "scripting.executeScript", "scripting.insertCSS", "scripting.removeCSS":
            guard let source = args["source"] as? String, !source.isEmpty else {
                return ["error": "No script source to execute."]
            }
            tab.realize().executeExtensionJavaScript(source, allFrames: allFrames)
            if (args["deferred"] as? NSNumber)?.boolValue == true {
                return ["deferred": true, "result": NSNull()]
            }
            return ["result": [["frameId": 0, "result": NSNull()]]]
        default:
            return ["error": "Unsupported scripting method: \(method)"]
        }
    }

    func activateExtensionAction(extensionID: String) {
        guard !extensionID.isEmpty, let tab = selectedTab else { return }
        if sidePanelOpenOnActionClick[extensionID] == true,
           openExtensionSidePanel(extensionID: extensionID) {
            return
        }
        emitExtensionEvent("action.onClicked",
                           args: [extensionTabRecord(tab)],
                           extensionID: extensionID)
    }

    func activateExtensionCommand(_ command: ExtensionStore.CommandDescriptor) {
        if command.commandName == "_execute_action" {
            if ExtensionStore.shared.requestActionPopup(extensionID: command.extensionID,
                                                        reason: "command") {
                return
            }
            activateExtensionAction(extensionID: command.extensionID)
            return
        }
        emitExtensionEvent("commands.onCommand",
                           args: [command.commandName],
                           extensionID: command.extensionID)
    }

    private func extensionTab(for rawID: Any?) -> BrowserTab? {
        guard let number = rawID as? NSNumber else { return nil }
        let tabID = number.intValue
        return tabs.first { $0.extensionTabID == tabID }
    }

    private func extensionTabIDs(from raw: Any?) -> [Int] {
        if let number = raw as? NSNumber {
            return [number.intValue]
        }
        if let numbers = raw as? [NSNumber] {
            return numbers.map(\.intValue)
        }
        if let array = raw as? NSArray {
            return array.compactMap { ($0 as? NSNumber)?.intValue }
        }
        return []
    }

    private func extensionTabIndices(from raw: Any?) -> [Int] {
        if let number = raw as? NSNumber {
            return [number.intValue]
        }
        if let numbers = raw as? [NSNumber] {
            return numbers.map(\.intValue)
        }
        if let array = raw as? NSArray {
            return array.compactMap { ($0 as? NSNumber)?.intValue }
        }
        return []
    }

    private func extensionTabs(matching queryInfo: NSDictionary) -> [BrowserTab] {
        tabs.compactMap { tab in
            if let active = queryInfo["active"] as? NSNumber,
               active.boolValue != (tab.id == selectedTabID) {
                return nil
            }
            if let currentWindow = queryInfo["currentWindow"] as? NSNumber,
               currentWindow.boolValue == false {
                return nil
            }
            if let urlPatterns = extensionQueryPatterns(queryInfo["url"]),
               !urlPatterns.contains(where: { wildcard($0, matches: tab.urlString) }) {
                return nil
            }
            if let titlePattern = queryInfo["title"] as? String,
               !wildcard(titlePattern, matches: tab.title) {
                return nil
            }
            return tab
        }
    }

    private func extensionQueryPatterns(_ raw: Any?) -> [String]? {
        if let value = raw as? String { return [value] }
        if let values = raw as? [String] { return values }
        return nil
    }

    private func extensionTabRecord(_ tab: BrowserTab) -> NSDictionary {
        let index = tabs.firstIndex { $0.id == tab.id } ?? 0
        let active = tab.id == selectedTabID
        var record: [String: Any] = [
            "id": tab.extensionTabID,
            "index": index,
            "windowId": 1,
            "active": active,
            "highlighted": active,
            "selected": active,
            "pinned": pinnedTabIDs.contains(tab.id),
            "groupId": extensionTabGroupMembership[tab.id] ?? -1,
            "url": tab.urlString,
            "title": tab.title,
            "status": tab.isLoading ? "loading" : "complete"
        ]
        if let favicon = tab.faviconURL { record["favIconUrl"] = favicon }
        return record as NSDictionary
    }

    private func removeTabFromExtensionGroup(_ tabID: BrowserTab.ID,
                                             emitTabUpdate: Bool = true) {
        guard let groupID = extensionTabGroupMembership.removeValue(forKey: tabID) else {
            return
        }
        if emitTabUpdate, let tab = tab(for: tabID) {
            emitExtensionTabUpdated(tab, changeInfo: ["groupId": -1])
        }
        pruneEmptyExtensionTabGroup(groupID)
    }

    private func pruneEmptyExtensionTabGroup(_ groupID: Int) {
        guard let group = extensionTabGroups[groupID],
              !extensionTabGroupMembership.values.contains(groupID)
        else { return }
        extensionTabGroups.removeValue(forKey: groupID)
        emitExtensionEvent("tabGroups.onRemoved", args: [extensionTabGroupRecord(group)])
    }

    private func extensionTabGroupRecord(_ group: ExtensionTabGroup) -> NSDictionary {
        var record: [String: Any] = [
            "id": group.id,
            "collapsed": group.collapsed,
            "color": group.color,
            "windowId": group.windowId,
            "shared": group.shared
        ]
        if let title = group.title {
            record["title"] = title
        }
        return record as NSDictionary
    }

    private func extensionTabGroup(_ group: ExtensionTabGroup,
                                   matches queryInfo: NSDictionary) -> Bool {
        if let collapsed = queryInfo["collapsed"] as? NSNumber,
           collapsed.boolValue != group.collapsed {
            return false
        }
        if let color = queryInfo["color"] as? String,
           color != group.color {
            return false
        }
        if let shared = queryInfo["shared"] as? NSNumber,
           shared.boolValue != group.shared {
            return false
        }
        if let windowID = queryInfo["windowId"] as? NSNumber,
           windowID.intValue >= 0,
           windowID.intValue != group.windowId {
            return false
        }
        if let titlePattern = queryInfo["title"] as? String,
           !wildcard(titlePattern, matches: group.title ?? "") {
            return false
        }
        return true
    }

    private func extensionSessionRecord(_ session: ClosedTabSession) -> NSDictionary {
        [
            "lastModified": session.closedAt.timeIntervalSince1970,
            "tab": [
                "sessionId": session.sessionID,
                "windowId": 1,
                "index": 0,
                "url": session.url,
                "title": session.title,
                "active": false,
                "highlighted": false,
                "selected": false,
                "pinned": false,
                "status": "complete",
                "incognito": false
            ]
        ]
    }

    private func extensionWindowRecord(populate: Bool,
                                       focusedOverride: Bool? = nil) -> NSDictionary {
        let window = extensionHostWindow()
        let frame = window?.frame ?? .zero
        let state: String
        if window?.isMiniaturized == true {
            state = "minimized"
        } else if window?.styleMask.contains(.fullScreen) == true {
            state = "fullscreen"
        } else {
            state = "normal"
        }

        var record: [String: Any] = [
            "id": 1,
            "focused": focusedOverride ?? (window?.isKeyWindow ?? true),
            "top": Int(frame.origin.y.rounded()),
            "left": Int(frame.origin.x.rounded()),
            "width": Int(frame.size.width.rounded()),
            "height": Int(frame.size.height.rounded()),
            "incognito": false,
            "type": "normal",
            "state": state,
            "alwaysOnTop": window?.level.rawValue ?? 0 > NSWindow.Level.normal.rawValue
        ]
        if populate {
            record["tabs"] = tabs.map(extensionTabRecord)
        }
        return record as NSDictionary
    }

    private func extensionHostWindow() -> NSWindow? {
        NSApp.mainWindow
            ?? NSApp.keyWindow
            ?? NSApp.windows.first {
                $0.isVisible && !$0.isMiniaturized && $0.frame.width > 0 && $0.frame.height > 0
            }
            ?? NSApp.windows.first {
                $0.frame.width > 0 && $0.frame.height > 0
            }
    }

    private func extensionWindowCreateURLs(_ raw: Any?) -> [String] {
        if let value = raw as? String, !value.isEmpty { return [value] }
        if let values = raw as? [String] { return values.filter { !$0.isEmpty } }
        if let values = raw as? NSArray {
            return values.compactMap { $0 as? String }.filter { !$0.isEmpty }
        }
        return []
    }

    private func emitExtensionTabUpdated(_ tab: BrowserTab, changeInfo: [String: Any]) {
        guard tabs.contains(where: { $0.id == tab.id }) else { return }
        emitExtensionEvent("tabs.onUpdated", args: [
            tab.extensionTabID,
            changeInfo,
            extensionTabRecord(tab)
        ])
    }

    private func completeIdentityFlowIfNeeded(tab: BrowserTab, url: String) {
        guard let flow = pendingIdentityFlows[tab.id],
              url == flow.redirectBase || url.hasPrefix(flow.redirectBase) || url.hasPrefix(flow.redirectOrigin)
        else { return }
        pendingIdentityFlows.removeValue(forKey: tab.id)
        let tabID = tab.id
        DispatchQueue.main.async { [weak self] in
            MoriBrowserView.dispatchExtensionBridgeResponse([
                "requestId": flow.requestID,
                "extensionId": flow.extensionID,
                "result": url
            ])
            self?.closeTab(tabID, allowPinned: true)
        }
    }

    private func emitExtensionTabMoveEvents(before: [BrowserTab],
                                            movedIDs: Set<BrowserTab.ID>) {
        guard !movedIDs.isEmpty else { return }
        let beforeIndices = Dictionary(uniqueKeysWithValues: before.enumerated().map {
            ($0.element.id, $0.offset)
        })
        for tab in tabs where movedIDs.contains(tab.id) {
            guard let fromIndex = beforeIndices[tab.id],
                  let toIndex = tabs.firstIndex(where: { $0.id == tab.id }),
                  fromIndex != toIndex
            else { continue }
            emitExtensionEvent("tabs.onMoved", args: [
                tab.extensionTabID,
                [
                    "windowId": 1,
                    "fromIndex": fromIndex,
                    "toIndex": toIndex
                ]
            ])
        }
    }

    private func emitExtensionWebNavigation(_ name: String,
                                            tab: BrowserTab,
                                            details rawDetails: [String: Any]) {
        guard tabs.contains(where: { $0.id == tab.id }) else { return }
        var details = rawDetails
        details["tabId"] = tab.extensionTabID
        details["timeStamp"] = Date().timeIntervalSince1970 * 1000
        emitExtensionEvent(name, args: [details])
    }

    private func emitExtensionEvent(_ name: String,
                                    args: [Any],
                                    extensionID: String? = nil) {
        MoriBrowserView.dispatchExtensionEvent(name, args: args, forExtensionID: extensionID)
    }

    private func wildcard(_ pattern: String, matches value: String) -> Bool {
        let escaped = NSRegularExpression.escapedPattern(for: pattern)
            .replacingOccurrences(of: "\\*", with: ".*")
        return value.range(of: "^\(escaped)$", options: [.regularExpression, .caseInsensitive]) != nil
    }

    private func jsonLiteral(_ value: Any) -> String {
        let object: Any
        if value is NSNull || JSONSerialization.isValidJSONObject([value]) {
            object = [value]
        } else {
            object = [String(describing: value)]
        }
        guard let data = try? JSONSerialization.data(withJSONObject: object),
              let array = String(data: data, encoding: .utf8),
              array.count >= 2
        else { return "null" }
        return String(array.dropFirst().dropLast())
    }

    /// Send the active tab to the configured homepage.
    func goHome() {
        selectedTab?.load(settings.homepageURL)
    }

    func toggleSidebar() {
        withAnimation(Motion.snappy) { sidebarVisible.toggle() }
    }

    func toggleAIPanel() {
        withAnimation(Motion.reveal) { aiPanelVisible.toggle() }
    }

    @discardableResult
    func openExtensionSidePanel(extensionID: String) -> Bool {
        guard let url = sidePanelURL(forExtensionID: extensionID) else { return false }
        let title = ExtensionStore.shared.extensions.first { $0.id == extensionID }?.name ?? "Extension"
        withAnimation(Motion.reveal) {
            extensionSidePanelURL = url
            extensionSidePanelTitle = title
        }
        return true
    }

    func closeExtensionSidePanel() {
        withAnimation(Motion.reveal) {
            extensionSidePanelURL = nil
            extensionSidePanelTitle = nil
        }
    }

    private func sidePanelURL(forExtensionID extensionID: String) -> String? {
        let options = sidePanelOptions[extensionID] ?? ExtensionSidePanelOptions()
        guard options.enabled else { return nil }
        if let path = options.path, !path.isEmpty {
            if path.contains("://") { return path }
            return ExtensionStore.shared.extensionResourceURL(forExtensionID: extensionID, path: path)
        }
        return ExtensionStore.shared.sidePanelURL(forExtensionID: extensionID)
    }

    func toggleSettings() {
        settingsVisible.toggle()
    }

    func prepareForTermination() {
        // Make sure the cookie jar is written before we tear CEF down, so
        // sessions reliably survive the quit.
        saveSession()
        MoriPrivacy.flushCookies()
        for tab in tabs { tab.close() }
    }

    /// Clear browsing data: history (and optionally cookies / cache). Cookies and
    /// cache go through the native CEF global stores.
    func clearBrowsingData(history: Bool = true,
                           cookies: Bool = true,
                           cache: Bool = true,
                           downloads: Bool = false) {
        if history { HistoryStore.shared.clear() }
        if cookies { MoriPrivacy.clearCookies() }
        if cache { MoriPrivacy.clearCache() }
        if downloads { DownloadStore.shared.clearAllRecords() }
    }

    func handleExtensionBrowsingData(method: String, args: NSDictionary) -> NSDictionary {
        let dataToRemove = args["dataToRemove"] as? NSDictionary ?? [:]

        func wants(_ key: String) -> Bool {
            (dataToRemove[key] as? NSNumber)?.boolValue ?? false
        }

        let history: Bool
        let cookies: Bool
        let cache: Bool
        let downloads: Bool

        switch method {
        case "browsingData.remove":
            history = wants("history")
            cookies = wants("cookies")
            cache = wants("cache")
            downloads = wants("downloads")
        case "browsingData.removeHistory":
            history = true
            cookies = false
            cache = false
            downloads = false
        case "browsingData.removeCookies":
            history = false
            cookies = true
            cache = false
            downloads = false
        case "browsingData.removeCache":
            history = false
            cookies = false
            cache = true
            downloads = false
        case "browsingData.removeDownloads":
            history = false
            cookies = false
            cache = false
            downloads = true
        case "browsingData.removeFormData",
             "browsingData.removeLocalStorage",
             "browsingData.removePasswords",
             "browsingData.removePluginData":
            return ["result": NSNull()]
        default:
            return ["error": "Unsupported browsingData method: \(method)"]
        }

        clearBrowsingData(history: history, cookies: cookies, cache: cache, downloads: downloads)
        return ["result": NSNull()]
    }

    // MARK: - Pinned tabs & folders

    private func tab(for id: BrowserTab.ID) -> BrowserTab? {
        tabs.first { $0.id == id }
    }

    /// Tabs in the pinned grid (stale ids are skipped).
    var pinnedTabs: [BrowserTab] {
        pinnedTabIDs.compactMap { tab(for: $0) }
    }

    private var folderedIDs: Set<BrowserTab.ID> {
        Set(folders.flatMap { $0.tabIDs })
    }

    /// Open tabs that are neither pinned nor inside a folder.
    var looseTabs: [BrowserTab] {
        tabs.filter { !pinnedTabIDs.contains($0.id) && !folderedIDs.contains($0.id) }
    }

    func tabs(in folder: TabFolder) -> [BrowserTab] {
        folder.tabIDs.compactMap { tab(for: $0) }
    }

    func isPinned(_ id: BrowserTab.ID) -> Bool { pinnedTabIDs.contains(id) }

    func togglePin(_ id: BrowserTab.ID) {
        withAnimation(Motion.snappy) {
            if pinnedTabIDs.contains(id) {
                pinnedTabIDs.removeAll { $0 == id }
            } else {
                detachFromFolders(id)
                pinnedTabIDs.append(id)
            }
            scheduleSessionSave()
        }
    }

    // MARK: Folder management

    @discardableResult
    func addFolder(name: String = "New Folder") -> TabFolder {
        let folder = TabFolder(name: name, isExpanded: true)
        withAnimation(Motion.snappy) { folders.append(folder) }
        scheduleSessionSave()
        return folder
    }

    @discardableResult
    func addFolderForEditing(name: String = "New Folder") -> TabFolder {
        let folder = addFolder(name: name)
        folderIDPendingRename = folder.id
        return folder
    }

    func consumeFolderRenameRequest(for folderID: TabFolder.ID) {
        guard folderIDPendingRename == folderID else { return }
        folderIDPendingRename = nil
    }

    func toggleFolder(_ folderID: TabFolder.ID) {
        guard let idx = folders.firstIndex(where: { $0.id == folderID }) else { return }
        withAnimation(Motion.snappy) { folders[idx].isExpanded.toggle() }
        scheduleSessionSave()
    }

    func renameFolder(_ folderID: TabFolder.ID, to name: String) {
        guard let idx = folders.firstIndex(where: { $0.id == folderID }) else { return }
        folders[idx].name = name
        scheduleSessionSave()
    }

    /// Delete a folder; its tabs fall back into the loose list.
    func deleteFolder(_ folderID: TabFolder.ID) {
        withAnimation(Motion.snappy) {
            folders.removeAll { $0.id == folderID }
        }
        scheduleSessionSave()
    }

    /// Move a tab into a folder, removing it from any other folder / the pins.
    func addTab(_ tabID: BrowserTab.ID, toFolder folderID: TabFolder.ID) {
        guard let idx = folders.firstIndex(where: { $0.id == folderID }) else { return }
        withAnimation(Motion.snappy) {
            detachFromFolders(tabID)
            pinnedTabIDs.removeAll { $0 == tabID }
            folders[idx].tabIDs.append(tabID)
            folders[idx].isExpanded = true
        }
        scheduleSessionSave()
    }

    func removeTabFromFolders(_ tabID: BrowserTab.ID) {
        withAnimation(Motion.snappy) { detachFromFolders(tabID) }
        scheduleSessionSave()
    }

    private func detachFromFolders(_ tabID: BrowserTab.ID) {
        for i in folders.indices {
            folders[i].tabIDs.removeAll { $0 == tabID }
        }
    }
}

/// Normalizes URLs that must stay inside Mori before a tab is ever created.
enum MoriURLRewriter {
    static func rewrite(_ raw: String) -> String {
        guard var components = URLComponents(string: raw),
              components.scheme?.lowercased() ==
                ["chrome", "extension"].joined(separator: "-")
        else {
            return raw
        }
        components.scheme = "mori-extension"
        return components.string ?? raw
    }
}

/// Turns omnibox input into a navigable URL or a search, honoring the user's
/// configured homepage and default search engine.
enum URLInterpreter {
    private static let allowedSchemes: Set<String> = [
        "http", "https", "file", "about", "mori", "mori-extension",
        ["chrome", "extension"].joined(separator: "-")
    ]

    static func resolve(_ raw: String, settings: BrowserSettings) -> String {
        let text = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        if text.isEmpty { return settings.homepageURL }

        // Already has a scheme.
        if hasAllowedScheme(text) {
            return text
        }

        // Looks like an address without a scheme, including paths, ports,
        // localhost, IPv4, and bracketed IPv6 hosts. Local addresses default
        // to http since they rarely serve TLS.
        if looksLikeWebAddress(text) {
            return "\(defaultScheme(forAddress: text))://\(text)"
        }

        // Otherwise search with the configured engine.
        return settings.searchURL(for: text)
    }

    static func resolvesAsAddress(_ raw: String) -> Bool {
        let text = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return false }
        return hasAllowedScheme(text) || looksLikeWebAddress(text)
    }

    private static func hasAllowedScheme(_ text: String) -> Bool {
        guard let scheme = URLComponents(string: text)?.scheme?.lowercased() else {
            return false
        }
        return allowedSchemes.contains(scheme)
    }

    private static func looksLikeWebAddress(_ text: String) -> Bool {
        guard text.rangeOfCharacter(from: .whitespacesAndNewlines) == nil,
              text.rangeOfCharacter(from: .controlCharacters) == nil,
              !text.contains("://")
        else {
            return false
        }

        let authority = text.split(whereSeparator: { "/?#".contains($0) }).first.map(String.init) ?? ""
        guard !authority.isEmpty else { return false }

        if authority.lowercased() == "localhost" {
            return true
        }
        if authority.lowercased().hasPrefix("localhost:") {
            let port = String(authority.dropFirst("localhost:".count))
            return isValidPort(port)
        }

        if authority.hasPrefix("[") {
            guard let end = authority.firstIndex(of: "]") else { return false }
            let host = String(authority[authority.index(after: authority.startIndex)..<end])
            let suffix = authority[authority.index(after: end)...]
            guard suffix.isEmpty || suffix.first == ":" else { return false }
            if suffix.first == ":" {
                let port = String(suffix.dropFirst())
                guard isValidPort(port) else { return false }
            }
            return isIPv6Literal(host)
        }

        let parts = authority.split(separator: ":", maxSplits: 1, omittingEmptySubsequences: false)
        if parts.count == 2, !isValidPort(String(parts[1])) {
            return false
        }

        let host = parts.first.map(String.init) ?? ""
        guard !host.isEmpty else { return false }

        if isIPv4Literal(host) { return true }
        return isDomainName(host)
    }

    /// Picks the implicit scheme for a scheme-less address. Local addresses
    /// (localhost, *.localhost, loopback IPs) use http; everything else https.
    private static func defaultScheme(forAddress text: String) -> String {
        let authority = text.split(whereSeparator: { "/?#".contains($0) }).first.map(String.init) ?? ""
        let host: String
        if authority.hasPrefix("["), let end = authority.firstIndex(of: "]") {
            host = String(authority[authority.index(after: authority.startIndex)..<end])
        } else {
            host = authority.split(separator: ":", maxSplits: 1, omittingEmptySubsequences: false)
                .first.map(String.init) ?? ""
        }
        return isLocalHost(host) ? "http" : "https"
    }

    private static func isLocalHost(_ host: String) -> Bool {
        let lower = host.lowercased()
        if lower == "localhost" || lower.hasSuffix(".localhost") { return true }
        if lower == "::1" { return true }
        if isIPv4Literal(lower) { return lower.hasPrefix("127.") }
        return false
    }

    private static func isDomainName(_ host: String) -> Bool {
        let labels = host.split(separator: ".", omittingEmptySubsequences: false).map(String.init)
        guard labels.count >= 2,
              labels.allSatisfy({ !$0.isEmpty && $0.count <= 63 }),
              let tld = labels.last,
              tld.count >= 2,
              tld.rangeOfCharacter(from: .letters) != nil
        else {
            return false
        }

        let allowed = CharacterSet(charactersIn: "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-")
        return labels.allSatisfy { label in
            guard label.rangeOfCharacter(from: allowed.inverted) == nil else { return false }
            return !(label.hasPrefix("-") || label.hasSuffix("-"))
        }
    }

    private static func isIPv4Literal(_ host: String) -> Bool {
        let parts = host.split(separator: ".", omittingEmptySubsequences: false)
        guard parts.count == 4 else { return false }
        return parts.allSatisfy { part in
            guard let value = Int(part), (0...255).contains(value) else { return false }
            return String(value) == part || part == "0"
        }
    }

    private static func isValidPort(_ raw: String) -> Bool {
        guard let port = Int(raw), (0...65535).contains(port) else {
            return false
        }
        return String(port) == raw || raw == "0"
    }

    private static func isIPv6Literal(_ host: String) -> Bool {
        var hints = addrinfo(
            ai_flags: AI_NUMERICHOST,
            ai_family: AF_INET6,
            ai_socktype: SOCK_STREAM,
            ai_protocol: 0,
            ai_addrlen: 0,
            ai_canonname: nil,
            ai_addr: nil,
            ai_next: nil)
        var result: UnsafeMutablePointer<addrinfo>?
        defer {
            if let result { freeaddrinfo(result) }
        }
        return getaddrinfo(host, nil, &hints, &result) == 0
    }
}
