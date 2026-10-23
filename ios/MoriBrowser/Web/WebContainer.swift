import SwiftUI
import UIKit
import WebKit

struct WebContainer: UIViewRepresentable {
    let webView: WKWebView
    let isLoading: Bool

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeUIView(context: Context) -> Host {
        let host = Host()
        host.attach(webView)
        installRefresh(on: webView, coordinator: context.coordinator)
        context.coordinator.webView = webView
        return host
    }

    func updateUIView(_ host: Host, context: Context) {
        if host.webView !== webView {
            host.attach(webView)
            installRefresh(on: webView, coordinator: context.coordinator)
            context.coordinator.webView = webView
        }
        if !isLoading {
            webView.scrollView.refreshControl?.endRefreshing()
        }
    }

    private func installRefresh(on webView: WKWebView, coordinator: Coordinator) {
        guard webView.scrollView.refreshControl == nil else { return }
        let refresh = UIRefreshControl()
        refresh.addTarget(coordinator, action: #selector(Coordinator.reload), for: .valueChanged)
        webView.scrollView.refreshControl = refresh
    }

    final class Coordinator: NSObject {
        weak var webView: WKWebView?
        @objc func reload() { webView?.reload() }
    }

    final class Host: UIView {
        weak var webView: WKWebView?
        func attach(_ webView: WKWebView) {
            self.webView?.removeFromSuperview()
            self.webView = webView
            webView.frame = bounds
            webView.autoresizingMask = [.flexibleWidth, .flexibleHeight]
            addSubview(webView)
        }
    }
}
