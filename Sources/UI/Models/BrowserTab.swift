import SwiftUI
import AppKit

/// One browser tab. Owns a native `MoriBrowserView` (a live CEF browser) and
/// republishes its navigation state for SwiftUI. The native view is created
/// lazily so background/unopened tabs stay cheap.
final class BrowserTab: NSObject, ObservableObject, Identifiable {
    let id: UUID
    let extensionTabID: Int
    private static var nextExtensionTabID = 1

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
    var onExtensionTabUpdated: ((BrowserTab, [String: Any]) -> Void)?
    var onExtensionNavigationEvent: ((String, BrowserTab, [String: Any]) -> Void)?

    private(set) lazy var browserView: MoriBrowserView = {
        let view = MoriBrowserView(url: urlString)
        view.extensionTabID = extensionTabID
        view.navDelegate = self
        return view
    }()

    private var isRealized = false

    init(id: UUID = UUID(), url: String, title: String = "New Tab") {
        self.id = id
        extensionTabID = Self.nextExtensionTabID
        Self.nextExtensionTabID += 1
        self.urlString = url
        self.title = title
        super.init()
    }

    /// Force the native view (and CEF browser) into existence.
    @discardableResult
    func realize() -> MoriBrowserView {
        isRealized = true
        return browserView
    }

    var hasRealized: Bool { isRealized }

    // MARK: Navigation passthrough

    func load(_ url: String) {
        let target = MoriURLRewriter.rewrite(url)
        urlString = target
        didFail = false
        onExtensionTabUpdated?(self, ["url": target, "status": "loading"])
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
    func startDownload(url: String, extensionID: String, requestID: String, filename: String?) -> Bool {
        realize().startDownload(url, extensionID: extensionID, requestID: requestID, filename: filename)
    }

    func captureVisiblePNGDataURL(extensionID: String, requestID: String) -> Bool {
        guard hasRealized else { return false }
        return browserView.captureVisiblePNGDataURL(forExtensionID: extensionID, requestID: requestID)
    }

    func evaluateJavaScript(_ source: String) async throws -> Any {
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

            let started = view.evaluateJavaScript(source) { result, errorMessage in
                if let errorMessage, !errorMessage.isEmpty {
                    resumeOnce(.failure(BrowserAutomationError.pageScriptFailed(errorMessage)))
                    return
                }
                resumeOnce(.success(result ?? NSNull()))
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
        if isRealized {
            browserView.closeBrowser()
        }
    }
}

// MARK: - MoriBrowserViewDelegate

extension BrowserTab: MoriBrowserViewDelegate {
    func browserView(_ view: MoriBrowserView, didChangeTitle title: String) {
        self.title = title.isEmpty ? "Untitled" : title
        HistoryStore.shared.updateTitle(self.title, for: urlString)
        onExtensionTabUpdated?(self, ["title": self.title])
    }

    func browserView(_ view: MoriBrowserView, didChangeURL url: String) {
        self.urlString = url
        HistoryStore.shared.record(url: url, title: title)
        onExtensionTabUpdated?(self, ["url": url])
    }

    func browserView(_ view: MoriBrowserView,
                     didChangeLoading isLoading: Bool,
                     canGoBack: Bool,
                     canGoForward: Bool) {
        if isLoading {
            didFail = false
        }
        self.isLoading = isLoading
        self.canGoBack = canGoBack
        self.canGoForward = canGoForward
        onExtensionTabUpdated?(self, ["status": isLoading ? "loading" : "complete"])
    }

    func browserView(_ view: MoriBrowserView, didChangeFaviconURLs urls: [String]) {
        self.faviconURL = urls.first
        if let faviconURL {
            onExtensionTabUpdated?(self, ["favIconUrl": faviconURL])
        }
    }

    func browserView(_ view: MoriBrowserView, didLoadFaviconImage image: NSImage?) {
        self.faviconImage = image
    }

    func browserView(_ view: MoriBrowserView,
                     didStartNavigationToURL url: String,
                     isRedirect: Bool,
                     userGesture: Bool) {
        // Drop the previous page's decoded icon so we don't show a stale favicon
        // for the new page; Chromium re-delivers one once it resolves.
        self.faviconImage = nil
        onExtensionNavigationEvent?("webNavigation.onBeforeNavigate", self, [
            "url": url,
            "frameId": 0,
            "parentFrameId": -1,
            "isRedirect": isRedirect,
            "userGesture": userGesture
        ])
    }

    func browserView(_ view: MoriBrowserView, didCommitNavigationToURL url: String) {
        onExtensionNavigationEvent?("webNavigation.onCommitted", self, [
            "url": url,
            "frameId": 0,
            "parentFrameId": -1,
            "transitionType": "link",
            "transitionQualifiers": []
        ])
    }

    func browserView(_ view: MoriBrowserView,
                     didFinishNavigationToURL url: String,
                     httpStatusCode: Int) {
        let details: [String: Any] = [
            "url": url,
            "frameId": 0,
            "parentFrameId": -1,
            "statusCode": httpStatusCode
        ]
        onExtensionNavigationEvent?("webNavigation.onDOMContentLoaded", self, details)
        onExtensionNavigationEvent?("webNavigation.onCompleted", self, details)
    }

    func browserView(_ view: MoriBrowserView,
                     didFailLoad errorText: String,
                     failedURL: String) {
        self.didFail = true
        onExtensionNavigationEvent?("webNavigation.onErrorOccurred", self, [
            "url": failedURL,
            "frameId": 0,
            "parentFrameId": -1,
            "error": errorText
        ])
    }

    func browserView(_ view: MoriBrowserView, requestsNewTabWithURL url: String) {
        onRequestNewTab?(url)
    }

    func browserView(_ view: MoriBrowserView,
                     didUpdateFindMatchOrdinal ordinal: Int32,
                     ofMatches count: Int32) {
        self.findOrdinal = Int(ordinal)
        self.findCount = Int(count)
    }
}
