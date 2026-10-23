import Observation
import SwiftUI
import WebKit

enum Destination: Identifiable, Hashable {
    case settings
    case library
    case space(UUID)

    var id: String {
        switch self {
        case .settings: return "settings"
        case .library: return "library"
        case .space(let id): return id.uuidString
        }
    }
}

struct SharedLink: Identifiable {
    let id = UUID()
    let url: URL
}

@MainActor
@Observable
final class BrowserStore {
    private(set) var workspace: Workspace
    private(set) var sessions: [UUID: WebSession] = [:]
    private var persistence: SessionStore?
    private var privateStores: [UUID: WKWebsiteDataStore] = [:]
    private var saveTask: Task<Void, Never>?
    private var toastTask: Task<Void, Never>?
    private var closedStack: [ClosedTab] = []

    var bottomBarHidden = false
    var commandBarPresented = false
    var commandToken = 0
    var sidebarPresented = false
    var findPresented = false
    var findText = ""
    var toast: String?
    var pageSheet: Destination?
    var sidebarDestination: Destination?
    var confirmClearToday = false
    var pendingDeleteSpace: UUID?
    var creatingFolder = false
    var folderName = ""
    var sharedLink: SharedLink?
    var readerArticle: ReaderArticle?
    var downloads: [String] = []

    init(workspace: Workspace? = nil, persistence: SessionStore? = nil) {
        if let workspace {
            self.workspace = workspace
            self.persistence = persistence
            return
        }
        #if DEBUG
        let arguments = ProcessInfo.processInfo.arguments
        if arguments.contains("-MoriSample") {
            var sample = Workspace.sample()
            if arguments.contains("-MoriSkipOnboarding") { sample.hasOnboarded = true }
            self.workspace = sample
            self.persistence = nil
            if arguments.contains("-MoriSidebar") { sidebarPresented = true }
            return
        }
        #endif
        let store = persistence ?? .live()
        self.persistence = store
        var loaded = store.load()
        #if DEBUG
        if ProcessInfo.processInfo.arguments.contains("-MoriSkipOnboarding") {
            loaded.hasOnboarded = true
        }
        #endif
        self.workspace = loaded
    }

    var activeSpace: Space { workspace.activeSpace }
    var selectedTabID: UUID? { workspace.activeSpace.selectedTabID }

    var activeSession: WebSession? {
        guard let id = selectedTabID else { return nil }
        return sessions[id]
    }

    var currentURL: URL? {
        if let url = activeSession?.webView.url, let scheme = url.scheme?.lowercased(), scheme == "http" || scheme == "https" {
            return url
        }
        guard let id = selectedTabID else { return nil }
        return workspace.tabs[id]?.resolvedURL
    }

    var hasPage: Bool { currentURL != nil }

    var addressTitle: String {
        guard let url = currentURL else { return "Search or enter address" }
        return url.host ?? url.absoluteString
    }

    var addressIsPlaceholder: Bool { currentURL == nil }

    var currentIsBookmarked: Bool {
        guard let url = currentURL?.absoluteString else { return false }
        return workspace.bookmarks.contains { $0.url == url }
    }

    var openCount: Int { activeSpace.tabIDs.count }

    func chromeScheme(system: ColorScheme) -> ColorScheme { system }

    var preferredScheme: ColorScheme? { nil }

    func canDelete(_ space: Space) -> Bool {
        space.isPrivate || workspace.spaces.filter { !$0.isPrivate }.count > 1
    }

    func ensureSelectedSession() {
        guard let id = selectedTabID else { return }
        ensureSession(for: id, loadIfNeeded: true)
    }

    func beginNavigation() {
        commandToken += 1
        sidebarPresented = false
        bottomBarHidden = false
        commandBarPresented = true
    }

    func setBottomBarHidden(_ hidden: Bool) {
        let next = hidden && hasPage
        guard bottomBarHidden != next else { return }
        bottomBarHidden = next
    }

    func newTab() {
        let id = workspace.newTab(in: workspace.activeSpaceID)
        ensureSession(for: id, loadIfNeeded: false)
        findPresented = false
        beginNavigation()
        scheduleSave()
    }

    func select(tab id: UUID) {
        workspace.select(tab: id)
        ensureSelectedSession()
        sidebarPresented = false
        bottomBarHidden = false
        findPresented = false
        scheduleSave()
    }

    func switchTo(_ id: UUID) {
        guard id != workspace.activeSpaceID else { return }
        workspace.switchTo(id)
        ensureSelectedSession()
        findPresented = false
        scheduleSave()
    }

    func close(tab id: UUID) {
        guard let closed = workspace.close(tab: id) else { return }
        closedStack.append(closed)
        if closedStack.count > 20 { closedStack.removeFirst() }
        sessions[id]?.abandonDialogs()
        sessions[id] = nil
        ensureSelectedSession()
        scheduleSave()
    }

    func closeSelected() {
        guard let id = selectedTabID else { return }
        close(tab: id)
    }

    func reopenClosed() {
        guard let closed = closedStack.popLast() else { return }
        workspace.reopen(closed)
        ensureSelectedSession()
        scheduleSave()
    }

    func clearToday() {
        let closed = workspace.clearToday(in: workspace.activeSpaceID)
        closedStack.append(contentsOf: closed)
        if closedStack.count > 20 {
            closedStack.removeFirst(closedStack.count - 20)
        }
        for item in closed {
            sessions[item.record.id]?.abandonDialogs()
            sessions[item.record.id] = nil
        }
        ensureSelectedSession()
        scheduleSave()
    }

    @discardableResult
    func addSpace(private isPrivate: Bool) -> UUID {
        let id = workspace.addSpace(private: isPrivate)
        ensureSelectedSession()
        if isPrivate {
            showToast("Private. This space is erased when Mori quits.")
        }
        scheduleSave()
        return id
    }

    func delete(space id: UUID) {
        let owned = workspace.spaces.first { $0.id == id }?.tabIDs ?? []
        guard workspace.deleteSpace(id) else { return }
        for tab in owned {
            sessions[tab]?.abandonDialogs()
            sessions[tab] = nil
        }
        privateStores[id] = nil
        pendingDeleteSpace = nil
        ensureSelectedSession()
        scheduleSave()
    }

    func rename(space id: UUID, to name: String) {
        workspace.rename(space: id, to: name)
        scheduleSave()
    }

    func setSymbol(_ symbol: String, for spaceID: UUID) {
        workspace.setSymbol(symbol, for: spaceID)
        scheduleSave()
    }

    func setTheme(_ theme: GradientTheme?, for spaceID: UUID) {
        workspace.setTheme(theme, for: spaceID)
        scheduleSave()
    }

    func toggleFavorite(tab id: UUID? = nil) {
        guard let id = id ?? selectedTabID else { return }
        if workspace.space(containing: id)?.isPrivate == true {
            showToast("Private tabs stay in this space")
            return
        }
        if workspace.favorites.contains(id) {
            workspace.unfavorite(tab: id)
        } else if !workspace.favorite(tab: id) {
            showToast("Favorites is full")
            return
        }
        ensureSelectedSession()
        scheduleSave()
    }

    func togglePin(tab id: UUID) {
        if workspace.space(containing: id)?.isPrivate == true {
            showToast("Private tabs stay in this space")
            return
        }
        if workspace.isPinned(id) {
            workspace.unpin(tab: id)
        } else {
            workspace.pin(tab: id)
        }
        scheduleSave()
    }

    func move(tab id: UUID, toFolder folderID: UUID) {
        guard workspace.move(tab: id, toFolder: folderID) else { return }
        scheduleSave()
    }

    func toggleFolder(_ id: UUID) {
        workspace.toggleFolder(id)
        scheduleSave()
    }

    func createFolder() {
        let name = folderName
        folderName = ""
        _ = workspace.addFolder(named: name, in: workspace.activeSpaceID)
        scheduleSave()
    }

    func deleteFolder(_ id: UUID) {
        workspace.deleteFolder(id)
        scheduleSave()
    }

    func open(url: URL, newTab: Bool = false) {
        if newTab || selectedTabID == nil {
            _ = workspace.newTab(in: workspace.activeSpaceID)
        }
        guard let id = selectedTabID else { return }
        workspace.navigate(tab: id, to: url)
        ensureSession(for: id, loadIfNeeded: false)
        sessions[id]?.load(url)
        sidebarPresented = false
        findPresented = false
        scheduleSave()
    }

    func commit(_ hit: CommandHit) {
        commandBarPresented = false
        switch hit.kind {
        case .navigate(let url), .search(_, let url):
            open(url: url)
        case .tab(let id):
            select(tab: id)
        case .bookmark(let id):
            guard let string = workspace.bookmarks.first(where: { $0.id == id })?.url,
                  let url = URL(string: string) else { return }
            open(url: url)
        case .history(let id):
            guard let string = workspace.history.first(where: { $0.id == id })?.url,
                  let url = URL(string: string) else { return }
            open(url: url)
        case .suggestion(let phrase):
            open(url: workspace.settings.searchEngine.searchURL(for: phrase))
        case .action(let action):
            perform(action)
        }
    }

    func open(_ destination: Destination, embedded: Bool) {
        commandBarPresented = false
        if embedded {
            pageSheet = nil
            sidebarDestination = destination
            return
        }
        sidebarDestination = nil
        guard sidebarPresented else {
            pageSheet = destination
            return
        }
        sidebarPresented = false
        Task { @MainActor in
            try? await Task.sleep(for: .milliseconds(280))
            pageSheet = destination
        }
    }

    func goBack() { activeSession?.goBack() }
    func goForward() { activeSession?.goForward() }
    func reload() { activeSession?.reload() }
    func stop() { activeSession?.stop() }

    func openFind() {
        guard hasPage else { return }
        findPresented = true
    }

    func closeFind() {
        findPresented = false
    }

    func find(forward: Bool) {
        let text = findText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        activeSession?.find(text, forward: forward)
    }

    func openReader() {
        guard let session = activeSession, hasPage else {
            showToast("Open a page first")
            return
        }
        Task { [weak self] in
            guard let article = await session.extractArticle() else {
                self?.showToast("This page doesn't have an article")
                return
            }
            self?.readerArticle = article
        }
    }

    func toggleBookmark() {
        guard let url = currentURL else { return }
        let title = workspace.tabs[selectedTabID ?? UUID()]?.displayTitle ?? url.host ?? "Bookmark"
        let added = workspace.toggleBookmark(title: title, url: url.absoluteString)
        showToast(added ? "Bookmark added" : "Bookmark removed")
        scheduleSave()
    }

    func removeBookmark(_ id: UUID) {
        workspace.removeBookmark(id)
        scheduleSave()
    }

    func removeHistory(_ id: UUID) {
        workspace.removeHistory(id)
        scheduleSave()
    }

    func copyLink() {
        guard let url = currentURL else { return }
        UIPasteboard.general.string = url.absoluteString
        showToast("Link copied")
    }

    func shareCurrent() {
        guard let url = currentURL else { return }
        sharedLink = SharedLink(url: url)
    }

    func adjustZoom(by delta: Double) {
        guard let session = activeSession else { return }
        let next = (min(2, max(0.5, session.webView.pageZoom + delta)) * 20).rounded() / 20
        session.webView.pageZoom = next
        guard let host = currentURL?.host else { return }
        workspace.settings.zoomByHost[host] = next
        scheduleSave()
    }

    func resetZoom() {
        guard let session = activeSession else { return }
        session.webView.pageZoom = 1
        if let host = currentURL?.host {
            workspace.settings.zoomByHost[host] = nil
        }
        scheduleSave()
    }

    func setAppearance(_ appearance: Appearance) {
        workspace.settings.appearance = appearance
        scheduleSave()
    }

    func setSearchEngine(_ engine: SearchEngine) {
        workspace.settings.searchEngine = engine
        scheduleSave()
    }

    func clearBrowsingData() {
        workspace.clearHistory()
        WKWebsiteDataStore.default().removeData(
            ofTypes: WKWebsiteDataStore.allWebsiteDataTypes(),
            modifiedSince: .distantPast
        ) {}
        URLCache.shared.removeAllCachedResponses()
        FaviconLoader.shared.clear()
        scheduleSave()
        showToast("Browsing data cleared")
    }

    func finishOnboarding() {
        guard !workspace.hasOnboarded else { return }
        workspace.hasOnboarded = true
        scheduleSave()
    }

    func saveNow() {
        saveTask?.cancel()
        persistence?.save(workspace)
    }

    func showToast(_ text: String) {
        toast = text
        toastTask?.cancel()
        toastTask = Task { [weak self] in
            try? await Task.sleep(for: .seconds(1.8))
            guard let self, !Task.isCancelled else { return }
            self.toast = nil
        }
    }

    func refreshDownloads() {
        downloads = DownloadDirectory.filenames()
    }

    func noteDownload(filename: String) {
        refreshDownloads()
        showToast("Saved \(filename)")
    }

    func deleteDownload(_ name: String) {
        try? FileManager.default.removeItem(at: DownloadDirectory.url.appendingPathComponent(name))
        refreshDownloads()
    }

    func noteVisit(tab id: UUID, url: URL, title: String?, recordsHistory: Bool) {
        let isPrivate = workspace.space(containing: id)?.isPrivate
            ?? (workspace.activeSpace.isPrivate && id == selectedTabID)
        workspace.noteVisit(tab: id, url: url, title: title, isPrivate: isPrivate, recordsHistory: recordsHistory)
        scheduleSave()
    }

    func applyZoom(to session: WebSession) {
        guard let host = session.webView.url?.host,
              let zoom = workspace.settings.zoomByHost[host] else { return }
        session.webView.pageZoom = zoom
    }

    func makePopup(configuration: WKWebViewConfiguration, action: WKNavigationAction) -> WKWebView? {
        let id = workspace.newTab(in: workspace.activeSpaceID)
        let session = WebSession(id: id, configuration: configuration, owner: self)
        session.markStarted()
        sessions[id] = session
        sidebarPresented = false
        scheduleSave()
        return session.webView
    }

    private func ensureSession(for id: UUID, loadIfNeeded: Bool) {
        if sessions[id] == nil {
            let space = workspace.space(containing: id) ?? workspace.activeSpace
            let session = WebSession(
                id: id,
                configuration: WebSession.configuration(dataStore: dataStore(for: space)),
                owner: self
            )
            sessions[id] = session
        }
        guard loadIfNeeded, let session = sessions[id], !session.hasCommitted else { return }
        guard let url = workspace.tabs[id]?.resolvedURL else { return }
        session.load(url)
    }

    private func dataStore(for space: Space) -> WKWebsiteDataStore {
        guard space.isPrivate else { return .default() }
        if let existing = privateStores[space.id] { return existing }
        let created = WKWebsiteDataStore.nonPersistent()
        privateStores[space.id] = created
        return created
    }

    private func perform(_ action: BrowserAction) {
        switch action {
        case .newTab:
            newTab()
        case .newSpace:
            _ = addSpace(private: false)
        case .privateSpace:
            _ = addSpace(private: true)
        case .reader:
            openReader()
        case .find:
            openFind()
        case .library:
            open(.library, embedded: false)
        case .settings:
            open(.settings, embedded: false)
        case .bookmark:
            toggleBookmark()
        case .copyLink:
            copyLink()
        }
    }

    private func scheduleSave() {
        guard persistence != nil else { return }
        saveTask?.cancel()
        saveTask = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(350))
            guard let self, !Task.isCancelled else { return }
            self.persistence?.save(self.workspace)
        }
    }
}
