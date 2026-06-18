import SwiftUI
import AppKit

/// One browser tab. Owns a native `MoriBrowserView` (a live CEF browser) and
/// republishes its navigation state for SwiftUI. The native view is created
/// lazily so background/unopened tabs stay cheap.
final class BrowserTab: NSObject, ObservableObject, Identifiable {
    let id: UUID

    @Published var title: String
    @Published var urlString: String
    @Published var isLoading: Bool = false
    @Published var canGoBack: Bool = false
    @Published var canGoForward: Bool = false
    @Published var faviconURL: String?
    /// The real site favicon, downloaded and decoded by Chromium (any format).
    /// Preferred over `faviconURL` for display; `faviconURL` remains the
    /// fallback (and the value reported to extensions / persisted).
    @Published var faviconImage: NSImage?
    @Published var didFail: Bool = false
    /// Human-readable failure reason from the last failed navigation, surfaced
    /// in the error overlay. Cleared whenever a new load begins.
    @Published var failError: String = ""

    /// Find-in-page results for the active query (1-based active match, total).
    @Published var findOrdinal: Int = 0
    @Published var findCount: Int = 0

    /// Page zoom as a percentage (100 = default). Tracked on the Swift side so
    /// the chrome can show it: CEF zoom is logarithmic (factor = 1.2^level) and
    /// every zoom command routes through the methods below, so mirroring the
    /// level here stays in sync without a native readback.
    @Published private(set) var zoomPercent: Int = 100
    /// Mirrors `kZoomStep` in MoriBrowserView.mm.
    private static let zoomStep = 0.5
    private var zoomLevel: Double = 0 {
        didSet { zoomPercent = Int((pow(1.2, zoomLevel) * 100).rounded()) }
    }

    /// The address shown in the omnibox while the user is *not* editing it.
    var displayURL: String {
        if urlString == "about:blank" { return "" }
        if urlString.hasPrefix("mori://") { return "" }
        return urlString
    }

    /// Callback set by the store so a tab can request opening a sibling tab
    /// (popups / target=_blank).
    var onRequestNewTab: ((String) -> Void)?
    /// Fired when persisted metadata (URL, title, favicon) changes so the
    /// store can schedule a session save.
    var onMetadataChanged: ((BrowserTab) -> Void)?
    /// Fired on each main-frame navigation commit so the store can apply Air
    /// Traffic Control routing rules.
    var onDidNavigate: ((BrowserTab, String) -> Void)?

    /// The native CEF-backed view. Created lazily on first `realize()` and
    /// recreated transparently after `sleep()` discards it to reclaim memory.
    /// Only ever touched for realized tabs, so reading it never forces an
    /// unwanted CEF browser into existence behind the caller's back.
    var browserView: MoriBrowserView {
        if let existing = _browserView { return existing }
        let view = MoriBrowserView(url: urlString)
        view.navDelegate = self
        _browserView = view
        return view
    }
    private var _browserView: MoriBrowserView?

    private var isRealized = false
    private var desiredChromePinnedState = false

    /// True while the tab is "asleep": its CEF browser has been discarded to
    /// free memory. It keeps its URL/title/favicon and reloads the last URL when
    /// next selected (`realize()` recreates the native view).
    @Published private(set) var isAsleep: Bool = false

    /// When the user last viewed/interacted with this tab. Drives the
    /// auto-sleep and auto-archive maintenance passes.
    @Published var lastAccessedAt: Date = Date()

    /// True while this tab is showing the distraction-free Reader view.
    @Published private(set) var readerActive: Bool = false

    /// True while the page is producing audible sound.
    @Published private(set) var isAudible: Bool = false
    /// True while this tab's audio is muted.
    @Published private(set) var isMuted: Bool = false

    /// Mute or unmute this tab's audio.
    func toggleMute() {
        guard isRealized else { return }
        let muted = !isMuted
        browserView.setAudioMuted(muted)
        isMuted = muted
    }

    /// Toggle Reader Mode: extract + restyle the article, or reload to restore.
    func toggleReader() {
        guard isRealized else { return }
        if readerActive {
            readerActive = false
            reload()
            return
        }
        Task { @MainActor in
            let ok = (try? await evaluateJavaScript(ReaderScripts.enable)) as? Bool ?? false
            if ok {
                readerActive = true
            } else {
                ToastCenter.shared.show("No article found to read",
                                        icon: "doc.plaintext", style: .warning)
            }
        }
    }

    init(id: UUID = UUID(), url: String, title: String = "New Tab") {
        self.id = id
        self.urlString = url
        self.title = title
        super.init()
    }

    /// Force the native view (and CEF browser) into existence, waking a sleeping
    /// tab in the process.
    @discardableResult
    func realize() -> MoriBrowserView {
        isRealized = true
        if isAsleep { isAsleep = false }
        browserView.setTabPinned(desiredChromePinnedState)
        return browserView
    }

    var hasRealized: Bool { isRealized }

    /// Record that the user just viewed / interacted with this tab.
    func markAccessed() {
        lastAccessedAt = Date()
    }

    /// Discard the live CEF browser to reclaim memory while keeping the tab in
    /// the sidebar. Reloads the last URL transparently on next `realize()`.
    /// No-op for an unrealized or already-sleeping tab.
    func sleep() {
        guard isRealized, let view = _browserView else { return }
        view.closeBrowser()
        _browserView = nil
        isRealized = false
        isAsleep = true
        isLoading = false
        canGoBack = false
        canGoForward = false
        findOrdinal = 0
        findCount = 0
        isAudible = false
        isMuted = false
    }

    func setChromePinned(_ pinned: Bool) {
        desiredChromePinnedState = pinned
        if isRealized {
            browserView.setTabPinned(pinned)
        }
    }

    // MARK: Navigation passthrough

    func load(_ url: String) {
        let target = MoriURLRewriter.rewrite(url)
        urlString = target
        didFail = false
        failError = ""
        markAccessed()
        onMetadataChanged?(self)
        realize().loadURL(target)
    }

    func goBack() { browserView.goBack() }
    func goForward() { browserView.goForward() }
    func reload() {
        didFail = false
        browserView.reload()
    }
    func reloadIgnoringCache() {
        didFail = false
        browserView.reloadIgnoringCache()
    }
    func stop() { browserView.stopLoading() }
    func focus() { browserView.focusBrowser() }

    // MARK: Site Boosts (per-site CSS/JS + zaps)

    /// Apply this page's site Boost (custom CSS/JS + zapped elements), if any.
    /// Idempotent: safe to call on every commit/finish callback.
    func applyBoosts() {
        guard isRealized,
              let script = BoostStore.shared.injectionScript(forURL: urlString)
        else { return }
        Task { @MainActor in _ = try? await evaluateJavaScript(script) }
    }

    /// Inject the click-to-zap element picker overlay into the live page.
    func injectZapPicker() {
        guard isRealized else { return }
        Task { @MainActor in _ = try? await evaluateJavaScript(BoostScripts.zapPicker) }
    }

    /// Tear down the zap picker and return the selectors the user clicked.
    func collectZaps() async -> [String] {
        guard isRealized,
              let result = try? await evaluateJavaScript(BoostScripts.collectZaps)
        else { return [] }
        return (result as? [Any])?.compactMap { $0 as? String } ?? []
    }

    // MARK: Link/image context menu

    /// Install the page-side contextmenu listener that captures the link/image
    /// under the cursor and suppresses Chrome's native menu when we'll show ours.
    func installContextMenuHook() {
        guard isRealized else { return }
        Task { @MainActor in _ = try? await evaluateJavaScript(WebContextScripts.listener) }
    }

    /// Install the media agent that powers the sidebar player and PiP. Idempotent
    /// per document in Mori's isolated media world; `BrowserStore` polls
    /// `window.__moriMediaState()` from that same world afterwards.
    func installMediaAgent() {
        guard isRealized else { return }
        Task { @MainActor in _ = try? await evaluateMediaJavaScript(MediaAgentScripts.agent) }
    }

    /// Read (and clear) the most recent right-click target, if it was a link or
    /// image.
    func readContextMenuTarget() async -> LinkImageContextTarget? {
        guard isRealized,
              let result = try? await evaluateJavaScript(WebContextScripts.read),
              let dict = result as? [String: Any]
        else { return nil }
        let link = (dict["link"] as? String) ?? ""
        let image = (dict["image"] as? String) ?? ""
        let target = LinkImageContextTarget(
            linkURL: link.isEmpty ? nil : link,
            linkText: (dict["linkText"] as? String) ?? "",
            imageURL: image.isEmpty ? nil : image,
            selection: (dict["selection"] as? String) ?? "")
        return target.hasContent ? target : nil
    }

    /// Copy the image under `windowPoint` (window coordinates) to the pasteboard
    /// via Chromium's native pipeline. Operates on the already-decoded bitmap,
    /// so it's immune to page CORS. Returns false when there's no live browser.
    func copyImage(at windowPoint: CGPoint) -> Bool {
        guard isRealized else { return false }
        return realize().copyImage(atWindowPoint: windowPoint)
    }

    /// Save the image under `windowPoint`. http(s) images download by URL
    /// through Chromium's download UI; canvas / data-URL images route through
    /// the renderer at the point. Returns false when the browser is unavailable.
    @discardableResult
    func saveImage(url: String, at windowPoint: CGPoint) -> Bool {
        guard isRealized else { return false }
        return realize().saveImageURL(url, atWindowPoint: windowPoint)
    }

    @MainActor
    func evaluateJavaScript(_ source: String) async throws -> Any {
        try await evaluateJavaScript(source, inMediaWorld: false)
    }

    @MainActor
    func evaluateMediaJavaScript(_ source: String) async throws -> Any {
        try await evaluateJavaScript(source, inMediaWorld: true)
    }

    @MainActor
    private func evaluateJavaScript(_ source: String, inMediaWorld: Bool) async throws -> Any {
        let view = realize()
        return try await withCheckedThrowingContinuation { continuation in
            var didResume = false
            func resumeOnce(_ result: Result<Any, Error>) {
                guard !didResume else { return }
                didResume = true
                switch result {
                case .success(let value):
                    continuation.resume(returning: value)
                case .failure(let error):
                    continuation.resume(throwing: error)
                }
            }

            let completion: (Any?, String?) -> Void = { result, errorMessage in
                if let errorMessage, !errorMessage.isEmpty {
                    resumeOnce(.failure(BrowserAutomationError.pageScriptFailed(errorMessage)))
                    return
                }
                resumeOnce(.success(result ?? NSNull()))
            }
            let started: Bool
            if inMediaWorld {
                started = view.evaluateMediaJavaScript(source, completion: completion)
            } else {
                started = view.evaluateJavaScript(source, completion: completion)
            }
            if !started {
                resumeOnce(.failure(BrowserAutomationError.browserUnavailable))
            }
        }
    }

    func zoomIn() { zoomLevel += Self.zoomStep; browserView.zoomIn() }
    func zoomOut() { zoomLevel -= Self.zoomStep; browserView.zoomOut() }
    func resetZoom() { zoomLevel = 0; browserView.resetZoom() }
    func setZoomFactor(_ factor: Double) {
        let safeFactor = min(max(factor, 0.25), 5.0)
        zoomLevel = log(safeFactor) / log(1.2)
        realize().setZoomFactor(safeFactor)
    }

    // MARK: Find-in-page / devtools / print

    func find(_ text: String, forward: Bool = true) {
        browserView.findText(text, forward: forward)
    }

    func stopFind() {
        browserView.stopFinding(true)
        findOrdinal = 0
        findCount = 0
    }

    func showDevTools() { browserView.showDevTools() }
    func toggleDevTools() { browserView.toggleDevTools() }
    func printPage() { browserView.printPage() }

    func close() {
        _browserView?.closeBrowser()
    }
}

// MARK: - MoriBrowserViewDelegate

extension BrowserTab: MoriBrowserViewDelegate {
    private func updateURL(_ url: String) {
        guard !url.isEmpty, url != urlString else { return }
        urlString = url
        HistoryStore.shared.record(url: url, title: title)
        onMetadataChanged?(self)
    }

    func browserView(_ view: MoriBrowserView, didChangeTitle title: String) {
        self.title = title.isEmpty ? "Untitled" : title
        HistoryStore.shared.updateTitle(self.title, for: urlString)
        onMetadataChanged?(self)
    }

    func browserView(_ view: MoriBrowserView, didChangeURL url: String) {
        updateURL(url)
    }

    func browserView(_ view: MoriBrowserView,
                     didChangeLoading isLoading: Bool,
                     canGoBack: Bool,
                     canGoForward: Bool) {
        if isLoading {
            didFail = false
            failError = ""
        }
        self.isLoading = isLoading
        self.canGoBack = canGoBack
        self.canGoForward = canGoForward
    }

    func browserView(_ view: MoriBrowserView, didChangeFaviconURLs urls: [String]) {
        self.faviconURL = urls.first
        if faviconURL != nil {
            onMetadataChanged?(self)
        }
    }

    func browserView(_ view: MoriBrowserView, didLoadFaviconImage image: NSImage?) {
        self.faviconImage = image
    }

    func browserView(_ view: MoriBrowserView,
                     didStartNavigationToURL url: String,
                     isRedirect: Bool,
                     userGesture: Bool) {
        // Drop the previous page's icon (both the decoded image and its URL) so
        // we don't show a stale favicon for the new page; Chromium re-delivers
        // one once it resolves.
        self.faviconImage = nil
        self.faviconURL = nil
        // A real navigation leaves the Reader view behind.
        if readerActive, !isRedirect { readerActive = false }
    }

    func browserView(_ view: MoriBrowserView, didCommitNavigationToURL url: String) {
        updateURL(url)
        // Inject CSS early to minimize flash; JS waits for the finish callback.
        applyBoosts()
        onDidNavigate?(self, url)
    }

    func browserView(_ view: MoriBrowserView,
                     didFinishNavigationToURL url: String,
                     httpStatusCode: Int) {
        updateURL(url)
        applyBoosts()
        installContextMenuHook()
        installMediaAgent()
    }

    func browserView(_ view: MoriBrowserView,
                     didFailLoad errorText: String,
                     failedURL: String) {
        self.failError = errorText.trimmingCharacters(in: .whitespacesAndNewlines)
        self.didFail = true
    }

    func browserView(_ view: MoriBrowserView, requestsNewTabWithURL url: String) {
        onRequestNewTab?(url)
    }

    func browserView(_ view: MoriBrowserView, didChangeAudioState audible: Bool) {
        self.isAudible = audible
    }

    func browserView(_ view: MoriBrowserView,
                     didUpdateFindMatchOrdinal ordinal: Int32,
                     ofMatches count: Int32) {
        self.findOrdinal = Int(ordinal)
        self.findCount = Int(count)
    }
}
