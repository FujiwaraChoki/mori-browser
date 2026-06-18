import SwiftUI

/// A tab that was archived (auto-closed after going stale, or archived by hand).
/// Kept restorable, Arc-style, so an aggressive archive policy never loses work.
struct ArchivedTab: Identifiable, Codable {
    var id = UUID()
    var url: String
    var title: String
    var faviconURL: String?
    var archivedAt: Date
}

/// Persistent archive of closed-by-staleness tabs, stored as JSON in Application
/// Support. Newest first, capped so it can't grow without bound.
final class ArchiveStore: ObservableObject {
    static let shared = ArchiveStore()

    @Published private(set) var tabs: [ArchivedTab] = []

    private let fileURL: URL
    private var saveScheduled = false
    private let limit = 300

    init() {
        let fm = FileManager.default
        let base = fm.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
            ?? fm.temporaryDirectory
        let dir = base.appendingPathComponent("MoriBrowser", isDirectory: true)
        try? fm.createDirectory(at: dir, withIntermediateDirectories: true)
        fileURL = dir.appendingPathComponent("archive.json")
        load()
    }

    /// Record an archived page. Skips blanks and collapses duplicate URLs to the
    /// most recent entry so re-archiving a recurring tab doesn't pile up.
    func add(url: String, title: String, faviconURL: String?) {
        guard !url.isEmpty, url != "about:blank", !url.hasPrefix("mori://") else { return }
        tabs.removeAll { $0.url == url }
        tabs.insert(ArchivedTab(url: url,
                                title: title.isEmpty ? url : title,
                                faviconURL: faviconURL,
                                archivedAt: Date()),
                    at: 0)
        if tabs.count > limit { tabs.removeLast(tabs.count - limit) }
        scheduleSave()
    }

    func remove(_ tab: ArchivedTab) {
        tabs.removeAll { $0.id == tab.id }
        scheduleSave()
    }

    func clear() {
        guard !tabs.isEmpty else { return }
        tabs.removeAll()
        scheduleSave()
    }

    // MARK: Persistence

    private func load() {
        guard let data = try? Data(contentsOf: fileURL),
              let decoded = try? JSONDecoder().decode([ArchivedTab].self, from: data)
        else { return }
        tabs = decoded
    }

    private func scheduleSave() {
        guard !saveScheduled else { return }
        saveScheduled = true
        DispatchQueue.main.async { [weak self] in
            self?.saveScheduled = false
            self?.save()
        }
    }

    private func save() {
        guard let data = try? JSONEncoder().encode(tabs) else { return }
        try? data.write(to: fileURL, options: .atomic)
    }
}
