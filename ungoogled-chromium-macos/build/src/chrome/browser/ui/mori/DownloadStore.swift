import SwiftUI
import AppKit

/// One in-flight or finished download. Mirrors the `CefDownloadItem` snapshot
/// broadcast by the native download handler.
struct DownloadItem: Identifiable {
    let id: UInt32
    var url: String
    var filename: String
    var path: String
    var received: Int64
    var total: Int64
    var percent: Int          // -1 when the total size is unknown.
    var speed: Int64          // bytes/sec
    var isComplete: Bool
    var isCanceled: Bool
    var isInProgress: Bool

    var fractionComplete: Double {
        if percent >= 0 { return Double(percent) / 100.0 }
        guard total > 0 else { return 0 }
        return Double(received) / Double(total)
    }

    var displayName: String {
        if !filename.isEmpty { return filename }
        return (path as NSString).lastPathComponent
    }

    /// "1.2 MB of 4.5 MB" / "3.1 MB" depending on what we know.
    var sizeSummary: String {
        let recv = ByteCountFormatter.string(fromByteCount: received, countStyle: .file)
        if total > 0 {
            let tot = ByteCountFormatter.string(fromByteCount: total, countStyle: .file)
            return "\(recv) of \(tot)"
        }
        return recv
    }

    var statusText: String {
        if isComplete { return "Completed" }
        if isCanceled { return "Canceled" }
        if speed > 0 {
            let rate = ByteCountFormatter.string(fromByteCount: speed, countStyle: .file)
            return "\(sizeSummary) — \(rate)/s"
        }
        return sizeSummary
    }
}

/// Observes the native `MoriDownloadUpdated` broadcast and maintains the list
/// of downloads for the Downloads panel. App-global (downloads aren't per-tab).
final class DownloadStore: ObservableObject {
    static let shared = DownloadStore()

    @Published private(set) var items: [DownloadItem] = []
    /// Bumped whenever a new download starts, so the chrome can flash the
    /// Downloads button.
    @Published private(set) var activityToken = 0
    /// Bumped whenever a download transitions to finished, so the chrome can
    /// surface the Downloads popover without the user hunting for it.
    @Published private(set) var completionToken = 0

    static let didUpdate = Notification.Name("MoriDownloadUpdated")

    private var observer: NSObjectProtocol?

    init() {
        observer = NotificationCenter.default.addObserver(
            forName: DownloadStore.didUpdate, object: nil, queue: .main
        ) { [weak self] note in
            self?.ingest(note.userInfo ?? [:])
        }
    }

    deinit {
        if let observer { NotificationCenter.default.removeObserver(observer) }
    }

    var activeItems: [DownloadItem] {
        items.filter { $0.isInProgress && !$0.isComplete && !$0.isCanceled }
    }

    var hasActiveDownloads: Bool { !activeItems.isEmpty }

    /// Combined progress of all in-flight downloads, 0...1. Drives the ring on
    /// the Downloads button.
    var aggregateFraction: Double {
        let active = activeItems
        guard !active.isEmpty else { return 0 }
        let total = active.reduce(0.0) { $0 + min(max($1.fractionComplete, 0), 1) }
        return total / Double(active.count)
    }

    /// True when at least one active download has no known size, so the ring
    /// should spin rather than fill to a fraction.
    var hasIndeterminateActive: Bool {
        activeItems.contains { $0.percent < 0 && $0.total <= 0 }
    }

    private func ingest(_ info: [AnyHashable: Any]) {
        guard let id = (info["id"] as? NSNumber)?.uint32Value else { return }
        let item = DownloadItem(
            id: id,
            url: info["url"] as? String ?? "",
            filename: info["filename"] as? String ?? "",
            path: info["path"] as? String ?? "",
            received: (info["received"] as? NSNumber)?.int64Value ?? 0,
            total: (info["total"] as? NSNumber)?.int64Value ?? 0,
            percent: (info["percent"] as? NSNumber)?.intValue ?? -1,
            speed: (info["speed"] as? NSNumber)?.int64Value ?? 0,
            isComplete: (info["complete"] as? NSNumber)?.boolValue ?? false,
            isCanceled: (info["canceled"] as? NSNumber)?.boolValue ?? false,
            isInProgress: (info["inProgress"] as? NSNumber)?.boolValue ?? false
        )

        if let idx = items.firstIndex(where: { $0.id == id }) {
            let previous = items[idx]
            items[idx] = item
            // Surface the popover the moment a transfer finishes.
            if !previous.isComplete && item.isComplete {
                completionToken &+= 1
            }
        } else {
            items.insert(item, at: 0)   // newest on top
            activityToken &+= 1
        }
    }

    // MARK: Actions

    func reveal(_ item: DownloadItem) {
        guard !item.path.isEmpty else { return }
        NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: item.path)])
    }

    /// Open a finished download with its default application. Extension packages
    /// (.crx) are handed to Mori's own installer rather than the OS, which would
    /// otherwise route them to whatever app owns the .crx type (typically Chrome).
    func open(_ item: DownloadItem) {
        guard item.isComplete, !item.path.isEmpty else { return }
        if (item.path as NSString).pathExtension.lowercased() == "crx" {
            MoriExtensionBridge.installCRX(atPath: item.path)
            return
        }
        NSWorkspace.shared.open(URL(fileURLWithPath: item.path))
    }

    func showDefaultFolder() {
        let downloads = FileManager.default.urls(for: .downloadsDirectory,
                                                 in: .userDomainMask).first
            ?? URL(fileURLWithPath: NSHomeDirectory())
        NSWorkspace.shared.open(downloads)
    }

    func clearFinished() {
        items.removeAll { $0.isComplete || $0.isCanceled }
    }

    func clearAllRecords() {
        items.removeAll()
    }

    func cancel(_ item: DownloadItem) {
        guard item.isInProgress, !item.isComplete, !item.isCanceled else { return }
        _ = MoriBrowserView.cancelDownload(withID: item.id)
    }
}
