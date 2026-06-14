import SwiftUI

/// One visited page.
struct HistoryEntry: Identifiable, Codable {
    var id = UUID()
    var url: String
    var title: String
    var lastVisited: Date
    var visitCount: Int
}

/// Persistent browsing history. Records main-frame navigations, collapses
/// repeat visits to the same URL, and is capped to a sane size. Stored as JSON
/// in Application Support so it survives relaunch (like the cookie jar).
final class HistoryStore: ObservableObject {
    static let shared = HistoryStore()

    @Published private(set) var entries: [HistoryEntry] = []

    private let maxEntries = 2000
    private let fileURL: URL
    private var saveScheduled = false

    init() {
        fileURL = HistoryStore.supportDirectory()
            .appendingPathComponent("history.json")
        load()
    }

    /// Record a visit. Ignores blanks and internal pages so the list stays
    /// meaningful. Same-URL revisits bump the count and move to the top.
    func record(url: String, title: String) {
        guard isRecordable(url) else { return }

        if let idx = entries.firstIndex(where: { $0.url == url }) {
            var updated = entries.remove(at: idx)
            updated.lastVisited = Date()
            updated.visitCount += 1
            if !title.isEmpty { updated.title = title }
            entries.insert(updated, at: 0)
        } else {
            let created = HistoryEntry(url: url, title: title,
                                       lastVisited: Date(), visitCount: 1)
            entries.insert(created, at: 0)
            if entries.count > maxEntries {
                entries.removeLast(entries.count - maxEntries)
            }
        }
        scheduleSave()
    }

    /// Update the title for the most recent entry of a URL (titles arrive after
    /// the navigation commits).
    func updateTitle(_ title: String, for url: String) {
        guard !title.isEmpty, let idx = entries.firstIndex(where: { $0.url == url })
        else { return }
        entries[idx].title = title
        scheduleSave()
    }

    /// Best prefix/substring matches for omnibox autocomplete, most-visited and
    /// most-recent first.
    func suggestions(for query: String, limit: Int = 6) -> [HistoryEntry] {
        let q = query.lowercased()
        guard !q.isEmpty else { return [] }
        return entries
            .filter { $0.url.lowercased().contains(q) || $0.title.lowercased().contains(q) }
            .sorted { ($0.visitCount, $0.lastVisited) > ($1.visitCount, $1.lastVisited) }
            .prefix(limit)
            .map { $0 }
    }

    func clear() {
        entries = []
        scheduleSave()
    }

    func remove(_ entry: HistoryEntry) {
        entries.removeAll { $0.id == entry.id }
        scheduleSave()
    }

    // MARK: Recordability

    private func isRecordable(_ url: String) -> Bool {
        guard !url.isEmpty, url != "about:blank" else { return false }
        let lower = url.lowercased()
        return !lower.hasPrefix("about:") && !lower.hasPrefix("chrome:")
            && !lower.hasPrefix("devtools:") && !lower.hasPrefix("data:")
    }

    // MARK: Persistence

    private static func supportDirectory() -> URL {
        let fm = FileManager.default
        let base = fm.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
            ?? fm.temporaryDirectory
        let dir = base.appendingPathComponent("MoriBrowser", isDirectory: true)
        try? fm.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    private func load() {
        guard let data = try? Data(contentsOf: fileURL),
              let decoded = try? JSONDecoder().decode([HistoryEntry].self, from: data)
        else { return }
        entries = decoded
    }

    /// Coalesce rapid navigations into a single write on the next runloop tick.
    private func scheduleSave() {
        guard !saveScheduled else { return }
        saveScheduled = true
        DispatchQueue.main.async { [weak self] in
            self?.saveScheduled = false
            self?.save()
        }
    }

    private func save() {
        guard let data = try? JSONEncoder().encode(entries) else { return }
        try? data.write(to: fileURL, options: .atomic)
    }
}
