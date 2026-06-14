import SwiftUI
import AppKit
import Combine

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

    // MARK: Contexts / pinned tabs / folders (sidebar organization)

    /// All contexts (Arc-style Spaces). Never empty — there is always at least
    /// one context, and `activeContextID` always points at a member.
    @Published var contexts: [BrowserContext] = []
    @Published var activeContextID: BrowserContext.ID = UUID()
    /// True while the sidebar shows the "Create a Context" flow instead of tabs.
    @Published var contextCreationVisible: Bool = false

    private var activeContextIndex: Int {
        contexts.firstIndex { $0.id == activeContextID } ?? 0
    }

    var activeContext: BrowserContext {
        get {
            contexts.first { $0.id == activeContextID }
                ?? contexts.first
                ?? BrowserContext(name: "Personal")
        }
        set {
            guard let idx = contexts.firstIndex(where: { $0.id == newValue.id }) else { return }
            contexts[idx] = newValue
        }
    }

    /// The active context's pinned tiles. Reads/writes pass through to the
    /// context so existing call sites keep their shape.
    var pinnedTabIDs: [BrowserTab.ID] {
        get { activeContext.pinnedTabIDs }
        set {
            guard contexts.indices.contains(activeContextIndex) else { return }
            contexts[activeContextIndex].pinnedTabIDs = newValue
        }
    }

    /// The active context's folders.
    var folders: [TabFolder] {
        get { activeContext.folders }
        set {
            guard contexts.indices.contains(activeContextIndex) else { return }
            contexts[activeContextIndex].folders = newValue
        }
    }

    /// Folder row that should enter rename mode as soon as it renders.
    @Published var folderIDPendingRename: TabFolder.ID?

    /// Mirrors theme edits (made through the existing pickers, which write
    /// `settings.gradientTheme`) back into the active context.
    private var themeMirror: AnyCancellable?

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
        // Legacy (pre-contexts) fields — optional so v2 files omit them and v1
        // files migrate into a single context on restore.
        var pinnedTabIDs: [UUID]? = nil
        var folders: [TabFolder]? = nil
        var spaceName: String? = nil
        var spaceEmoji: String? = nil
        // Contexts (v2).
        var contexts: [BrowserContext]? = nil
        var activeContextID: UUID? = nil
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
        ensureContextIntegrity()
        syncChromePinnedStates()
        // The active context's theme drives the chrome while it's active. On a
        // fresh (v1) profile the migration seeds it from the global theme, so
        // this is a no-op there.
        settings.gradientTheme = activeContext.theme
        isRestoringSession = false

        // Theme edits made anywhere (sidebar popover, Settings) write
        // `settings.gradientTheme`; fold them back into the active context so
        // each context keeps its own wash, Arc-style.
        themeMirror = settings.$gradientTheme
            .dropFirst()
            .sink { [weak self] theme in
                guard let self, !self.isRestoringSession else { return }
                guard self.contexts.indices.contains(self.activeContextIndex) else { return }
                guard self.contexts[self.activeContextIndex].theme != theme else { return }
                self.contexts[self.activeContextIndex].theme = theme
                self.scheduleSessionSave()
            }
        // Let the media controller map an engine broadcast back to its tab.
        media.resolveTab = { [weak self] browserId in
            self?.tabs.first {
                $0.hasRealized && Int($0.browserView.browserIdentifier) == browserId
            }
        }
        installExtensionCommandSmokeIfNeeded()
    }

    /// Smoke-test hook: fire extension keyboard commands shortly after launch
    /// so automated runs can verify command dispatch end to end.
    private func installExtensionCommandSmokeIfNeeded() {
        let env = ProcessInfo.processInfo.environment
        guard let extensionID = env["MORI_EXTENSION_SMOKE_COMMAND_ID"],
              !extensionID.isEmpty
        else { return }
        let commandName = env["MORI_EXTENSION_SMOKE_COMMAND_NAME"] ?? "_execute_action"
        let extraCommandName = env["MORI_EXTENSION_SMOKE_EXTRA_COMMAND_NAME"]
        func fire(_ name: String, after delay: TimeInterval) {
            DispatchQueue.main.asyncAfter(deadline: .now() + delay) {
                guard let command = ExtensionStore.shared.commands.first(where: {
                    $0.extensionID == extensionID && $0.commandName == name
                }) else { return }
                ExtensionStore.shared.activate(command)
            }
        }
        fire(commandName, after: 1.5)
        if let extraCommandName, !extraCommandName.isEmpty {
            fire(extraCommandName, after: 2.0)
        }
    }

    var selectedTab: BrowserTab? {
        tabs.first { $0.id == selectedTabID }
    }

    var shouldAutoFocusWebContent: Bool {
        !launcherVisible &&
            !settingsVisible &&
            !findBarVisible &&
            !contextCreationVisible &&
            folderIDPendingRename == nil
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

        if let persistedContexts = decoded.contexts, !persistedContexts.isEmpty {
            contexts = persistedContexts.map { context in
                var copy = context
                copy.tabIDs = copy.tabIDs.filter { liveIDs.contains($0) }
                copy.pinnedTabIDs = copy.pinnedTabIDs.filter { liveIDs.contains($0) }
                copy.folders = copy.folders.map { folder in
                    var f = folder
                    f.tabIDs = f.tabIDs.filter { liveIDs.contains($0) }
                    return f
                }
                if let sel = copy.selectedTabID, !liveIDs.contains(sel) {
                    copy.selectedTabID = nil
                }
                return copy
            }
            activeContextID = decoded.activeContextID
                .flatMap { id in contexts.first { $0.id == id }?.id }
                ?? contexts[0].id
        } else {
            // v1 session: fold the flat pinned/folder state into one context.
            let name = decoded.spaceName.flatMap { $0.isEmpty ? nil : $0 } ?? "Personal"
            let context = BrowserContext(
                name: name,
                symbol: "glyph-circle",
                theme: settings.gradientTheme,
                tabIDs: restoredTabs.map(\.id),
                pinnedTabIDs: (decoded.pinnedTabIDs ?? []).filter { liveIDs.contains($0) },
                folders: (decoded.folders ?? []).map { folder in
                    var copy = folder
                    copy.tabIDs = copy.tabIDs.filter { liveIDs.contains($0) }
                    return copy
                },
                selectedTabID: selectedTabID)
            contexts = [context]
            activeContextID = context.id
        }
        return true
    }

    /// Post-restore invariants: at least one context exists, the active id is
    /// valid, and every live tab belongs to exactly one context.
    private func ensureContextIntegrity() {
        if contexts.isEmpty {
            let context = BrowserContext(
                name: "Personal",
                symbol: "glyph-circle",
                theme: settings.gradientTheme,
                tabIDs: tabs.map(\.id),
                selectedTabID: selectedTabID)
            contexts = [context]
        }
        if !contexts.contains(where: { $0.id == activeContextID }) {
            activeContextID = contexts[0].id
        }
        var seen = Set<BrowserTab.ID>()
        for i in contexts.indices {
            contexts[i].tabIDs.removeAll { !seen.insert($0).inserted }
        }
        let orphans = tabs.map(\.id).filter { !seen.contains($0) }
        if !orphans.isEmpty {
            contexts[activeContextIndex].tabIDs.append(contentsOf: orphans)
        }
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
        var snapshot = contexts.map { context in
            var copy = context
            copy.tabIDs = copy.tabIDs.filter { liveIDs.contains($0) }
            copy.pinnedTabIDs = copy.pinnedTabIDs.filter { liveIDs.contains($0) }
            copy.folders = copy.folders.map { folder in
                var f = folder
                f.tabIDs = f.tabIDs.filter { liveIDs.contains($0) }
                return f
            }
            return copy
        }
        // Keep the active context's remembered selection fresh.
        if let idx = snapshot.firstIndex(where: { $0.id == activeContextID }) {
            snapshot[idx].selectedTabID = selectedTabID
        }
        let state = PersistedSession(
            tabs: tabs.map { PersistedTab(id: $0.id, url: $0.urlString, title: $0.title, faviconURL: $0.faviconURL) },
            selectedTabID: selectedTabID,
            contexts: snapshot,
            activeContextID: activeContextID
        )
        guard let data = try? JSONEncoder().encode(state) else { return }
        try? data.write(to: sessionFileURL, options: .atomic)
    }

    private func makeTab(id: BrowserTab.ID = UUID(), url: String, title: String) -> BrowserTab {
        let tab = BrowserTab(id: id, url: url, title: title)
        tab.onRequestNewTab = { [weak self] url in
            self?.newTab(url: url)
        }
        tab.onMetadataChanged = { [weak self] _ in
            self?.scheduleSessionSave()
        }
        return tab
    }

    // MARK: Tab management

    @discardableResult
    func newTab(url: String? = nil, select: Bool = true) -> BrowserTab {
        let initialURL = MoriURLRewriter.rewrite(url ?? settings.newTabURL)
        let tab = makeTab(url: initialURL, title: "New Tab")
        tabs.append(tab)
        addToActiveContext(tab.id)
        if select { selectTab(tab.id) }
        scheduleSessionSave()
        return tab
    }

    /// Register a freshly created tab as a member of the active context.
    private func addToActiveContext(_ id: BrowserTab.ID) {
        guard contexts.indices.contains(activeContextIndex) else { return }
        guard !contexts[activeContextIndex].tabIDs.contains(id) else { return }
        contexts[activeContextIndex].tabIDs.append(id)
    }

    // MARK: - Split view (Zen-style vertical split)

    enum SplitSide { case left, right }
    @Published var splitTabID: BrowserTab.ID?
    @Published var splitSide: SplitSide = .right

    /// Show `id` side-by-side with the selected tab (drag-from-sidebar drop).
    func splitWith(_ id: BrowserTab.ID, side: SplitSide) {
        guard id != selectedTabID, tabs.contains(where: { $0.id == id }) else {
            return
        }
        splitSide = side
        splitTabID = id
        tab(for: id)?.realize()
    }

    func closeSplit() {
        splitTabID = nil
    }

    func selectTab(_ id: BrowserTab.ID) {
        // Switching to a tab leaves the settings page — it covers the web card,
        // so it would otherwise stay parked over the newly selected tab.
        if settingsVisible { settingsVisible = false }
        // Selecting a tab that lives in another context (launcher jump,
        // extension activation) brings its context along, Arc-style.
        if let owner = contexts.firstIndex(where: { $0.tabIDs.contains(id) }),
           contexts[owner].id != activeContextID {
            switchContext(to: contexts[owner].id, selectRemembered: false)
        }
        let previous = selectedTabID
        selectedTabID = id
        selectedTab?.realize()
        selectedTab?.setChromePinned(isPinned(id))
        selectedTab?.focus()
        if previous != id { scheduleSessionSave() }
    }

    // MARK: New-tab launcher (command palette)

    /// Open the new-tab launcher instead of immediately creating a blank tab, so
    /// the user can search, jump to an open tab, or pick from history first.
    func presentLauncher() {
        launcherPrefill = ""
        launcherEditsCurrentTab = false
        NSLog("MORI-KEY presentLauncher visible %d->1", launcherVisible ? 1 : 0)
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
        NSLog("MORI-KEY toggleLauncher visible=%d main=%d",
              launcherVisible ? 1 : 0, Thread.isMainThread ? 1 : 0)
        if launcherVisible {
            dismissLauncher()
        } else {
            presentLauncher()
        }
    }

    func dismissLauncher() {
        launcherEditsCurrentTab = false
        NSLog("MORI-KEY dismissLauncher visible %d->0", launcherVisible ? 1 : 0)
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
        if id == splitTabID || id == selectedTabID { splitTabID = nil }
        guard let idx = tabs.firstIndex(where: { $0.id == id }) else { return }
        // Pinned tabs are permanent: a close gesture (Cmd-W, close button) is
        // ignored. They can only be removed by explicitly unpinning them.
        if contexts.contains(where: { $0.pinnedTabIDs.contains(id) }), !allowPinned { return }
        let tab = tabs[idx]
        // Remember where it was pointing so Cmd-Shift-T can bring it back.
        let url = tab.urlString
        if url != "about:blank", !url.isEmpty {
            closedTabSessions.append(ClosedTabSession(sessionID: UUID().uuidString,
                                                      url: url,
                                                      title: tab.title,
                                                      closedAt: Date()))
            if closedTabSessions.count > 25 { closedTabSessions.removeFirst() }
        }
        // Remember its position among the active context's visible tabs so the
        // next selection stays in place (and in this context).
        let contextOrder = orderedTabsForShortcuts.map(\.id)
        let closedContextIndex = contextOrder.firstIndex(of: id)

        tab.close()
        // Animate the removal so the sidebar row fades + shrinks and the rows
        // below collapse up into the gap, matching Zen's `animateItemClose`.
        withAnimation(Motion.tabClose) {
            tabs.remove(at: idx)
            for contextIndex in contexts.indices {
                contexts[contextIndex].tabIDs.removeAll { $0 == id }
                contexts[contextIndex].pinnedTabIDs.removeAll { $0 == id }
                for folderIndex in contexts[contextIndex].folders.indices {
                    contexts[contextIndex].folders[folderIndex].tabIDs.removeAll { $0 == id }
                }
                if contexts[contextIndex].selectedTabID == id {
                    contexts[contextIndex].selectedTabID = nil
                }
            }
        }
        if tabs.isEmpty {
            // Always keep at least one tab open.
            let fresh = newTab(select: true)
            selectedTabID = fresh.id
            return
        }
        if selectedTabID == id {
            let remaining = orderedTabsForShortcuts
            if remaining.isEmpty {
                // The active context just emptied; keep it alive with a fresh tab.
                _ = newTab(select: true)
            } else {
                let newIndex = min(closedContextIndex ?? remaining.count - 1,
                                   remaining.count - 1)
                selectTab(remaining[newIndex].id)
            }
        }
        scheduleSessionSave()
    }

    func moveTab(from source: IndexSet, to destination: Int) {
        tabs.move(fromOffsets: source, toOffset: destination)
        scheduleSessionSave()
    }

    @discardableResult
    func duplicateTab(_ id: BrowserTab.ID, select: Bool = true) -> BrowserTab? {
        guard let tab = tabs.first(where: { $0.id == id }) else { return nil }
        let duplicate = makeTab(url: tab.urlString, title: tab.title)
        duplicate.faviconURL = tab.faviconURL
        let sourceIndex = tabs.firstIndex { $0.id == id } ?? tabs.count - 1
        tabs.insert(duplicate, at: min(sourceIndex + 1, tabs.count))
        // The copy lands in the source tab's context, right after the original.
        if let owner = contexts.firstIndex(where: { $0.tabIDs.contains(id) }) {
            let at = contexts[owner].tabIDs.firstIndex(of: id).map { $0 + 1 }
                ?? contexts[owner].tabIDs.count
            contexts[owner].tabIDs.insert(duplicate.id, at: at)
        } else {
            addToActiveContext(duplicate.id)
        }
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
        let context = activeContext
        let ids = context.tabIDs
            .filter { $0 != id && !context.pinnedTabIDs.contains($0) }
        for tabID in ids {
            closeTab(tabID)
        }
        selectTab(id)
    }

    func closeTabsToRight(of id: BrowserTab.ID) {
        let context = activeContext
        guard let index = context.tabIDs.firstIndex(of: id),
              index + 1 < context.tabIDs.count
        else { return }
        let ids = context.tabIDs[(index + 1)...]
            .filter { !context.pinnedTabIDs.contains($0) }
        for tabID in ids {
            closeTab(tabID)
        }
        selectTab(id)
    }

    func hasClosableTabsToRight(of id: BrowserTab.ID) -> Bool {
        let context = activeContext
        guard let index = context.tabIDs.firstIndex(of: id),
              index + 1 < context.tabIDs.count
        else { return false }
        return context.tabIDs[(index + 1)...].contains { !context.pinnedTabIDs.contains($0) }
    }

    /// Reopen the most recently closed tab, restoring its last URL.
    func reopenClosedTab() {
        guard let session = closedTabSessions.popLast() else { return }
        _ = newTab(url: session.url, select: true)
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

    /// Tab order used by the Cmd-1…Cmd-9 shortcuts: the active context's pinned
    /// tabs first (matching the sidebar), then its remaining tabs in order.
    private var orderedTabsForShortcuts: [BrowserTab] {
        let context = activeContext
        let pinned = context.pinnedTabIDs.compactMap { tab(for: $0) }
        let rest = context.tabIDs
            .filter { !context.pinnedTabIDs.contains($0) }
            .compactMap { tab(for: $0) }
        return pinned + rest
    }

    /// Cycle to the next/previous tab in the active context, wrapping around.
    func selectNextTab() { cycleTab(by: 1) }
    func selectPreviousTab() { cycleTab(by: -1) }

    private func cycleTab(by delta: Int) {
        let ordered = orderedTabsForShortcuts
        guard ordered.count > 1,
              let current = ordered.firstIndex(where: { $0.id == selectedTabID })
        else { return }
        let next = (current + delta + ordered.count) % ordered.count
        selectTab(ordered[next].id)
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

    /// Send the active tab to the configured homepage.
    func goHome() {
        selectedTab?.load(settings.homepageURL)
    }

    func toggleSidebar() {
        let before = sidebarVisible
        withAnimation(Motion.snappy) { sidebarVisible.toggle() }
        NSLog("MORI-KEY store.toggleSidebar main=%d %d->%d",
              Thread.isMainThread ? 1 : 0, before ? 1 : 0, sidebarVisible ? 1 : 0)
    }

    func toggleAIPanel() {
        withAnimation(Motion.reveal) { aiPanelVisible.toggle() }
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

    /// The active context's tabs that are neither pinned nor inside a folder,
    /// in the context's sidebar order.
    var looseTabs: [BrowserTab] {
        let context = activeContext
        let foldered = folderedIDs
        return context.tabIDs
            .filter { !context.pinnedTabIDs.contains($0) && !foldered.contains($0) }
            .compactMap { tab(for: $0) }
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
            syncChromePinnedState(for: id)
            scheduleSessionSave()
        }
    }

    func syncChromePinnedStates() {
        let pinnedIDs = Set(contexts.flatMap(\.pinnedTabIDs))
        for tab in tabs {
            tab.setChromePinned(pinnedIDs.contains(tab.id))
        }
    }

    func syncChromePinnedState(for id: BrowserTab.ID) {
        tab(for: id)?.setChromePinned(contexts.contains {
            $0.pinnedTabIDs.contains(id)
        })
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

    /// Set a folder's glyph (the icon shown in the pocket). Empty/default is
    /// "folder", which renders the plain pocket with no inner glyph.
    func setFolderSymbol(_ folderID: TabFolder.ID, symbol: String) {
        guard let idx = folders.firstIndex(where: { $0.id == folderID }) else { return }
        folders[idx].symbol = symbol
        scheduleSessionSave()
    }

    // MARK: - Contexts (Arc-style Spaces)

    /// Switch the sidebar (and chrome theme) to another context. Remembers the
    /// outgoing context's selection and restores the destination's last selected
    /// tab (or its first tab; an empty context just shows its New Tab
    /// affordances).
    func switchContext(to id: BrowserContext.ID, selectRemembered: Bool = true) {
        guard id != activeContextID,
              let targetIndex = contexts.firstIndex(where: { $0.id == id })
        else { return }

        // Stash the outgoing context's state.
        if contexts.indices.contains(activeContextIndex) {
            contexts[activeContextIndex].selectedTabID = selectedTabID
            contexts[activeContextIndex].theme = settings.gradientTheme
        }

        withAnimation(Motion.state) {
            activeContextID = id
        }
        settings.gradientTheme = contexts[targetIndex].theme

        if selectRemembered {
            let context = contexts[targetIndex]
            let candidate = context.selectedTabID.flatMap { sel in
                context.tabIDs.contains(sel) ? sel : nil
            } ?? context.tabIDs.first
            if let candidate {
                selectTab(candidate)
            }
        }
        scheduleSessionSave()
    }

    /// Create a context and switch to it. Starts empty — the sidebar's New Tab
    /// affordances take it from there.
    @discardableResult
    func addContext(name: String, symbol: String, theme: GradientTheme = .none) -> BrowserContext {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        let context = BrowserContext(
            name: trimmed.isEmpty ? "Context \(contexts.count + 1)" : trimmed,
            symbol: symbol,
            theme: theme)
        withAnimation(Motion.snappy) { contexts.append(context) }
        switchContext(to: context.id)
        scheduleSessionSave()
        return context
    }

    /// Delete a context, closing its tabs. The last context can't be deleted —
    /// matching Arc, there is always at least one space.
    func deleteContext(_ id: BrowserContext.ID) {
        guard contexts.count > 1,
              let idx = contexts.firstIndex(where: { $0.id == id })
        else { return }

        if id == activeContextID {
            let fallback = contexts[idx == 0 ? 1 : idx - 1].id
            switchContext(to: fallback)
        }
        let doomedTabs = contexts[idx].tabIDs
        withAnimation(Motion.snappy) {
            contexts.removeAll { $0.id == id }
        }
        for tabID in doomedTabs {
            closeTab(tabID, allowPinned: true)
        }
        scheduleSessionSave()
    }

    func renameContext(_ id: BrowserContext.ID, to name: String) {
        guard let idx = contexts.firstIndex(where: { $0.id == id }) else { return }
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        contexts[idx].name = trimmed
        scheduleSessionSave()
    }

    func setContextSymbol(_ id: BrowserContext.ID, symbol: String) {
        guard let idx = contexts.firstIndex(where: { $0.id == id }) else { return }
        contexts[idx].symbol = symbol
        scheduleSessionSave()
    }

    func setContextTheme(_ id: BrowserContext.ID, theme: GradientTheme) {
        guard let idx = contexts.firstIndex(where: { $0.id == id }) else { return }
        contexts[idx].theme = theme
        if id == activeContextID {
            settings.gradientTheme = theme
        }
        scheduleSessionSave()
    }

    /// Open a fresh tab side-by-side with the current one (the plus menu's
    /// "New Split"). The new tab joins the sidebar like any other.
    func newSplit() {
        guard selectedTab != nil else {
            newTab()
            return
        }
        let tab = newTab(select: false)
        splitWith(tab.id, side: .right)
    }
}

/// Normalizes URLs before a tab is ever created. Chrome's own schemes
/// (including chrome-extension://) load natively, so this is currently a
/// pass-through kept for the call-site shape.
enum MoriURLRewriter {
    static func rewrite(_ raw: String) -> String { raw }
}

/// Turns omnibox input into a navigable URL or a search, honoring the user's
/// configured homepage and default search engine.
enum URLInterpreter {
    private static let allowedSchemes: Set<String> = [
        "http", "https", "file", "about", "mori", "chrome",
        "chrome-extension"
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
