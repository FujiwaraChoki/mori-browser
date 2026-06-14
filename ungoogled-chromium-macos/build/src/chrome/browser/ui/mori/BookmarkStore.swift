import SwiftUI

/// A saved bookmark.
struct Bookmark: Identifiable, Codable {
    var id = UUID()
    var url: String
    var title: String
    var createdAt: Date
}

/// Persistent bookmarks, stored as JSON in Application Support. Newest first.
final class BookmarkStore: ObservableObject {
    static let shared = BookmarkStore()

    @Published private(set) var bookmarks: [Bookmark] = []

    private let fileURL: URL
    private var saveScheduled = false

    init() {
        let fm = FileManager.default
        let base = fm.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
            ?? fm.temporaryDirectory
        let dir = base.appendingPathComponent("MoriBrowser", isDirectory: true)
        try? fm.createDirectory(at: dir, withIntermediateDirectories: true)
        fileURL = dir.appendingPathComponent("bookmarks.json")
        load()
    }

    func isBookmarked(_ url: String) -> Bool {
        bookmarks.contains { $0.url == url }
    }

    /// Add or remove the page from bookmarks. Returns the new bookmarked state.
    @discardableResult
    func toggle(url: String, title: String) -> Bool {
        guard !url.isEmpty, url != "about:blank" else { return false }
        if let idx = bookmarks.firstIndex(where: { $0.url == url }) {
            bookmarks.remove(at: idx)
            scheduleSave()
            return false
        }
        bookmarks.insert(Bookmark(url: url, title: title.isEmpty ? url : title,
                                  createdAt: Date()), at: 0)
        scheduleSave()
        return true
    }

    func remove(_ bookmark: Bookmark) {
        remove(id: bookmark.id.uuidString)
    }

    // MARK: Persistence

    private func load() {
        guard let data = try? Data(contentsOf: fileURL),
              let decoded = try? JSONDecoder().decode([Bookmark].self, from: data)
        else { return }
        bookmarks = decoded
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
        guard let data = try? JSONEncoder().encode(bookmarks) else { return }
        try? data.write(to: fileURL, options: .atomic)
    }

    @discardableResult
    private func remove(id: String) -> Bool {
        guard let idx = bookmarks.firstIndex(where: { $0.id.uuidString == id }) else {
            return false
        }
        bookmarks.remove(at: idx)
        scheduleSave()
        return true
    }
}
