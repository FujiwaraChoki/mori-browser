import UIKit
import WebKit

enum DialogResult {
    case ok(String?)
    case cancel
}

enum PageDialog: Equatable {
    case alert(String)
    case confirm(String)
    case prompt(String, String)

    var message: String {
        switch self {
        case .alert(let message), .confirm(let message), .prompt(let message, _):
            return message
        }
    }
}

enum DownloadDirectory {
    static var url: URL {
        let documents = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
        let directory = documents.appendingPathComponent("Downloads", isDirectory: true)
        try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        return directory
    }

    static func uniqueFile(suggestedFilename: String) -> URL {
        let cleaned = suggestedFilename
            .replacingOccurrences(of: "/", with: "-")
            .replacingOccurrences(of: ":", with: "-")
        let base = cleaned.isEmpty ? "Download" : cleaned
        var candidate = url.appendingPathComponent(base)
        let stem = candidate.deletingPathExtension().lastPathComponent
        let ext = candidate.pathExtension
        var suffix = 2
        while FileManager.default.fileExists(atPath: candidate.path) {
            let name = ext.isEmpty ? "\(stem)-\(suffix)" : "\(stem)-\(suffix).\(ext)"
            candidate = url.appendingPathComponent(name)
            suffix += 1
        }
        return candidate
    }

    static func filenames() -> [String] {
        let names = (try? FileManager.default.contentsOfDirectory(atPath: url.path)) ?? []
        return names.filter { !$0.hasPrefix(".") }.sorted()
    }
}

@MainActor
@Observable
final class WebSession {
    let id: UUID
    let webView: WKWebView
    weak var owner: BrowserStore?
    private let bridge: WebBridge
    private var observations: [NSKeyValueObservation] = []
    private var dialogHandler: ((DialogResult) -> Void)?
    var pendingDownloads: [ObjectIdentifier: String] = [:]

    var title = "New Tab"
    var isLoading = false
    var progress = 0.0
    var canGoBack = false
    var canGoForward = false
    var failure: String?
    var dialog: PageDialog?
    private(set) var hasCommitted = false
    private var lastScrollY: CGFloat = 0
    private var scrollDelta: CGFloat = 0

    init(id: UUID, configuration: WKWebViewConfiguration, owner: BrowserStore) {
        self.id = id
        self.owner = owner
        let webView = WKWebView(frame: .zero, configuration: configuration)
        webView.allowsBackForwardNavigationGestures = true
        webView.scrollView.keyboardDismissMode = .interactive
        webView.scrollView.contentInsetAdjustmentBehavior = .never
        webView.backgroundColor = .systemBackground
        webView.scrollView.backgroundColor = .systemBackground
        #if DEBUG
        webView.isInspectable = true
        #endif
        self.webView = webView
        bridge = WebBridge()
        bridge.session = self
        webView.navigationDelegate = bridge
        webView.uiDelegate = bridge
        observe()
    }

    static func configuration(dataStore: WKWebsiteDataStore) -> WKWebViewConfiguration {
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = dataStore
        configuration.allowsInlineMediaPlayback = true
        configuration.mediaTypesRequiringUserActionForPlayback = []
        configuration.preferences.javaScriptCanOpenWindowsAutomatically = true
        let preferences = WKWebpagePreferences()
        preferences.preferredContentMode = UIDevice.current.userInterfaceIdiom == .pad ? .desktop : .mobile
        configuration.defaultWebpagePreferences = preferences
        return configuration
    }

    func markStarted() {
        hasCommitted = true
    }

    func load(_ url: URL) {
        guard let scheme = url.scheme?.lowercased(), scheme == "http" || scheme == "https" else { return }
        hasCommitted = true
        failure = nil
        isLoading = true
        webView.load(URLRequest(url: url))
    }

    func reload() {
        failure = nil
        if webView.url == nil, let url = owner?.workspace.tabs[id]?.resolvedURL {
            load(url)
            return
        }
        webView.reload()
    }

    func stop() { webView.stopLoading() }
    func goBack() { if webView.canGoBack { webView.goBack() } }
    func goForward() { if webView.canGoForward { webView.goForward() } }

    func find(_ text: String, forward: Bool) {
        let configuration = WKFindConfiguration()
        configuration.backwards = !forward
        configuration.caseSensitive = false
        configuration.wraps = true
        webView.find(text, configuration: configuration) { _ in }
    }

    func present(_ dialog: PageDialog, completion: @escaping (DialogResult) -> Void) {
        if let existing = dialogHandler {
            dialogHandler = nil
            self.dialog = nil
            existing(.cancel)
        }
        self.dialog = dialog
        dialogHandler = completion
    }

    func dismissDialog(_ result: DialogResult) {
        guard let handler = dialogHandler else {
            dialog = nil
            return
        }
        dialog = nil
        dialogHandler = nil
        handler(result)
    }

    func abandonDialogs() {
        dismissDialog(.cancel)
    }

    fileprivate func handleStart() {
        isLoading = true
        failure = nil
        lastScrollY = webView.scrollView.contentOffset.y
        scrollDelta = 0
        owner?.setBottomBarHidden(false)
    }

    fileprivate func noteScroll(y: CGFloat, userDriven: Bool) {
        let delta = y - lastScrollY
        lastScrollY = y
        guard userDriven, abs(delta) < 160 else {
            scrollDelta = 0
            return
        }
        if y <= 4 {
            scrollDelta = 0
            owner?.setBottomBarHidden(false)
            return
        }
        scrollDelta += delta
        if scrollDelta > 16 {
            scrollDelta = 0
            owner?.setBottomBarHidden(true)
        } else if scrollDelta < -10 {
            scrollDelta = 0
            owner?.setBottomBarHidden(false)
        }
    }

    fileprivate func handleFinish() {
        isLoading = false
        progress = 1
        failure = nil
        canGoBack = webView.canGoBack
        canGoForward = webView.canGoForward
        if let pageTitle = webView.title, !pageTitle.isEmpty {
            title = pageTitle
        }
        guard let url = webView.url else { return }
        owner?.noteVisit(tab: id, url: url, title: webView.title, recordsHistory: true)
        owner?.applyZoom(to: self)
    }

    fileprivate func handleFailure(_ error: Error) {
        let ns = error as NSError
        if ns.domain == NSURLErrorDomain && ns.code == NSURLErrorCancelled { return }
        if ns.domain == "WKErrorDomain" && (ns.code == 102 || ns.code == 204) { return }
        failure = error.localizedDescription
        isLoading = false
    }

    private func observe() {
        observations = [
            webView.observe(\.estimatedProgress, options: [.new]) { [weak self] webView, _ in
                let value = webView.estimatedProgress
                Task { @MainActor in self?.progress = value }
            },
            webView.observe(\.isLoading, options: [.new]) { [weak self] webView, _ in
                let value = webView.isLoading
                Task { @MainActor in self?.isLoading = value }
            },
            webView.observe(\.canGoBack, options: [.new]) { [weak self] webView, _ in
                let value = webView.canGoBack
                Task { @MainActor in self?.canGoBack = value }
            },
            webView.observe(\.canGoForward, options: [.new]) { [weak self] webView, _ in
                let value = webView.canGoForward
                Task { @MainActor in self?.canGoForward = value }
            },
            webView.scrollView.observe(\.contentOffset, options: [.new]) { [weak self] scrollView, _ in
                let y = scrollView.contentOffset.y
                let userDriven = scrollView.isTracking || scrollView.isDragging || scrollView.isDecelerating
                MainActor.assumeIsolated {
                    self?.noteScroll(y: y, userDriven: userDriven)
                }
            },
            webView.observe(\.title, options: [.new]) { [weak self] webView, _ in
                let pageTitle = webView.title
                let url = webView.url
                Task { @MainActor in
                    guard let self else { return }
                    if let pageTitle, !pageTitle.isEmpty { self.title = pageTitle }
                    if let url {
                        self.owner?.noteVisit(tab: self.id, url: url, title: pageTitle, recordsHistory: false)
                    }
                }
            },
        ]
    }
}

private final class WebBridge: NSObject, WKNavigationDelegate, WKUIDelegate, WKDownloadDelegate {
    weak var session: WebSession?

    func webView(_ webView: WKWebView, didStartProvisionalNavigation navigation: WKNavigation!) {
        Task { @MainActor in session?.handleStart() }
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        Task { @MainActor in session?.handleFinish() }
    }

    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
        Task { @MainActor in session?.handleFailure(error) }
    }

    func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) {
        Task { @MainActor in session?.handleFailure(error) }
    }

    func webViewWebContentProcessDidTerminate(_ webView: WKWebView) {
        Task { @MainActor in session?.webView.reload() }
    }

    func webView(
        _ webView: WKWebView,
        decidePolicyFor navigationAction: WKNavigationAction,
        decisionHandler: @escaping (WKNavigationActionPolicy) -> Void
    ) {
        guard let url = navigationAction.request.url else {
            decisionHandler(.allow)
            return
        }
        let scheme = url.scheme?.lowercased() ?? ""
        switch scheme {
        case "http", "https", "about", "blob":
            decisionHandler(.allow)
        case "javascript", "file", "data":
            decisionHandler(.cancel)
        default:
            decisionHandler(.cancel)
            DispatchQueue.main.async {
                UIApplication.shared.open(url)
            }
        }
    }

    func webView(
        _ webView: WKWebView,
        decidePolicyFor navigationResponse: WKNavigationResponse,
        decisionHandler: @escaping (WKNavigationResponsePolicy) -> Void
    ) {
        decisionHandler(navigationResponse.canShowMIMEType ? .allow : .download)
    }

    func webView(_ webView: WKWebView, navigationAction: WKNavigationAction, didBecome download: WKDownload) {
        download.delegate = self
    }

    func webView(_ webView: WKWebView, navigationResponse: WKNavigationResponse, didBecome download: WKDownload) {
        download.delegate = self
    }

    func download(
        _ download: WKDownload,
        decideDestinationUsing response: URLResponse,
        suggestedFilename: String,
        completionHandler: @escaping (URL?) -> Void
    ) {
        let destination = DownloadDirectory.uniqueFile(suggestedFilename: suggestedFilename)
        let name = destination.lastPathComponent
        if Thread.isMainThread {
            MainActor.assumeIsolated {
                session?.pendingDownloads[ObjectIdentifier(download)] = name
            }
        }
        completionHandler(destination)
    }

    func downloadDidFinish(_ download: WKDownload) {
        let key = ObjectIdentifier(download)
        Task { @MainActor in
            let name = session?.pendingDownloads.removeValue(forKey: key) ?? "Download"
            session?.owner?.noteDownload(filename: name)
        }
    }

    func download(_ download: WKDownload, didFailWithError error: Error, resumeData: Data?) {
        let key = ObjectIdentifier(download)
        Task { @MainActor in
            session?.pendingDownloads.removeValue(forKey: key)
            session?.owner?.showToast("Couldn't download that file")
        }
    }

    func webView(
        _ webView: WKWebView,
        createWebViewWith configuration: WKWebViewConfiguration,
        for navigationAction: WKNavigationAction,
        windowFeatures: WKWindowFeatures
    ) -> WKWebView? {
        guard navigationAction.targetFrame == nil, Thread.isMainThread else { return nil }
        return MainActor.assumeIsolated {
            session?.owner?.makePopup(configuration: configuration, action: navigationAction)
        }
    }

    func webView(
        _ webView: WKWebView,
        requestMediaCapturePermissionFor origin: WKSecurityOrigin,
        initiatedByFrame frame: WKFrameInfo,
        type: WKMediaCaptureType,
        decisionHandler: @escaping (WKPermissionDecision) -> Void
    ) {
        decisionHandler(.prompt)
    }

    func webView(
        _ webView: WKWebView,
        runJavaScriptAlertPanelWithMessage message: String,
        initiatedByFrame frame: WKFrameInfo,
        completionHandler: @escaping () -> Void
    ) {
        Task { @MainActor in
            guard let session else {
                completionHandler()
                return
            }
            session.present(.alert(message)) { _ in completionHandler() }
        }
    }

    func webView(
        _ webView: WKWebView,
        runJavaScriptConfirmPanelWithMessage message: String,
        initiatedByFrame frame: WKFrameInfo,
        completionHandler: @escaping (Bool) -> Void
    ) {
        Task { @MainActor in
            guard let session else {
                completionHandler(false)
                return
            }
            session.present(.confirm(message)) { result in
                if case .ok = result { completionHandler(true) } else { completionHandler(false) }
            }
        }
    }

    func webView(
        _ webView: WKWebView,
        runJavaScriptTextInputPanelWithPrompt prompt: String,
        defaultText: String?,
        initiatedByFrame frame: WKFrameInfo,
        completionHandler: @escaping (String?) -> Void
    ) {
        Task { @MainActor in
            guard let session else {
                completionHandler(nil)
                return
            }
            session.present(.prompt(prompt, defaultText ?? "")) { result in
                if case .ok(let text) = result { completionHandler(text) } else { completionHandler(nil) }
            }
        }
    }
}
