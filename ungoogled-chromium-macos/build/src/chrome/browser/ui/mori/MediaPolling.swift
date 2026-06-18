import SwiftUI

/// Drives the sidebar media player and Picture-in-Picture by sampling each live
/// tab's injected media agent (see MediaAgentScripts.swift). Unlike the old CEF
/// build's push channel, the overlay pulls: a low-frequency timer reads
/// `window.__moriMediaState()` from every awake tab and rebroadcasts it as the
/// `MoriMediaUpdated` notification that `MediaController` already consumes.
extension BrowserStore {
    /// How often each live tab's media agent is sampled. Fast enough for the
    /// sidebar scrubber to feel live, coarse enough to stay cheap.
    private static let mediaPollInterval: TimeInterval = 1.0

    func startMediaPolling() {
        mediaPollTimer?.invalidate()
        let timer = Timer.scheduledTimer(withTimeInterval: Self.mediaPollInterval,
                                         repeats: true) { [weak self] _ in
            self?.pollMediaState()
        }
        timer.tolerance = 0.3
        mediaPollTimer = timer
    }

    /// Pull each awake tab's media snapshot and rebroadcast it. Asleep tabs are
    /// skipped so polling never resurrects a freed browser.
    private func pollMediaState() {
        for tab in tabs where tab.hasRealized && !tab.isAsleep {
            let browserId = Int(tab.browserView.browserIdentifier)
            Task { @MainActor in
                guard
                    let result = try? await tab.evaluateJavaScript(
                        "window.__moriMediaState ? window.__moriMediaState() : ''"),
                    let json = result as? String, !json.isEmpty
                else { return }
                NotificationCenter.default.post(
                    name: Notification.Name("MoriMediaUpdated"),
                    object: nil,
                    userInfo: ["browserId": browserId, "json": json])
            }
        }
    }
}
