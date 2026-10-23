import Foundation

enum BrowserAction: String, Equatable, CaseIterable, Identifiable {
    case newTab
    case newSpace
    case privateSpace
    case reader
    case find
    case library
    case settings
    case bookmark
    case copyLink

    var id: String { rawValue }

    var title: String {
        switch self {
        case .newTab: return "New Tab"
        case .newSpace: return "New Space"
        case .privateSpace: return "New Private Space"
        case .reader: return "Reader"
        case .find: return "Find in Page"
        case .library: return "Library"
        case .settings: return "Settings"
        case .bookmark: return "Bookmark This Page"
        case .copyLink: return "Copy Link"
        }
    }

    var symbol: String {
        switch self {
        case .newTab: return "plus"
        case .newSpace: return "square.split.2x1"
        case .privateSpace: return "lock"
        case .reader: return "text.alignleft"
        case .find: return "magnifyingglass"
        case .library: return "books.vertical"
        case .settings: return "gearshape"
        case .bookmark: return "bookmark"
        case .copyLink: return "link"
        }
    }
}

enum CommandSection: String, Equatable, CaseIterable {
    case best = "Go"
    case tabs = "Open Tabs"
    case bookmarks = "Bookmarks"
    case history = "History"
    case suggestions = "Suggestions"
    case actions = "Actions"
}

struct CommandHit: Equatable, Identifiable {
    enum Kind: Equatable {
        case navigate(URL)
        case search(String, URL)
        case tab(UUID)
        case bookmark(UUID)
        case history(UUID)
        case suggestion(String)
        case action(BrowserAction)
    }

    var id: String
    var title: String
    var subtitle: String?
    var symbol: String
    var section: CommandSection
    var kind: Kind
}

enum CommandIndex {
    static func hits(query raw: String, workspace: Workspace, suggestions: [String]) -> [CommandHit] {
        let query = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        if query.isEmpty {
            return emptyQuery(workspace)
        }
        var hits: [CommandHit] = []
        if let resolution = Omnibox.resolve(query, engine: workspace.settings.searchEngine) {
            switch resolution {
            case .url(let url):
                hits.append(CommandHit(
                    id: "go-\(url.absoluteString)",
                    title: url.host ?? url.absoluteString,
                    subtitle: url.absoluteString,
                    symbol: "globe",
                    section: .best,
                    kind: .navigate(url)
                ))
                let search = workspace.settings.searchEngine.searchURL(for: query)
                hits.append(CommandHit(
                    id: "search-\(query)",
                    title: "Search \(workspace.settings.searchEngine.label)",
                    subtitle: query,
                    symbol: "magnifyingglass",
                    section: .best,
                    kind: .search(query, search)
                ))
            case .search(let text, let url):
                hits.append(CommandHit(
                    id: "search-\(text)",
                    title: "Search \(workspace.settings.searchEngine.label)",
                    subtitle: text,
                    symbol: "magnifyingglass",
                    section: .best,
                    kind: .search(text, url)
                ))
            }
        }
        hits.append(contentsOf: matchingTabs(query, workspace: workspace))
        hits.append(contentsOf: matchingBookmarks(query, workspace: workspace))
        hits.append(contentsOf: matchingHistory(query, workspace: workspace))
        let existing = Set(hits.map { $0.title.lowercased() })
        for phrase in suggestions where !existing.contains(phrase.lowercased()) && phrase.lowercased() != query.lowercased() {
            hits.append(CommandHit(
                id: "suggest-\(phrase)",
                title: phrase,
                subtitle: workspace.settings.searchEngine.label,
                symbol: "magnifyingglass",
                section: .suggestions,
                kind: .suggestion(phrase)
            ))
        }
        return hits
    }

    private static func emptyQuery(_ workspace: Workspace) -> [CommandHit] {
        var hits = BrowserAction.allCases.filter { $0 != .newSpace && $0 != .privateSpace }.map { action in
            CommandHit(
                id: "action-\(action.rawValue)",
                title: action.title,
                subtitle: nil,
                symbol: action.symbol,
                section: .actions,
                kind: .action(action)
            )
        }
        let tabs = workspace.tabs.values.sorted { $0.displayTitle.localizedCaseInsensitiveCompare($1.displayTitle) == .orderedAscending }
        for tab in tabs.prefix(8) where tab.url != nil {
            hits.append(CommandHit(
                id: "tab-\(tab.id.uuidString)",
                title: tab.displayTitle,
                subtitle: WebAddress.host(of: tab.url),
                symbol: "square.on.square",
                section: .tabs,
                kind: .tab(tab.id)
            ))
        }
        for entry in workspace.history.prefix(6) {
            hits.append(CommandHit(
                id: "history-\(entry.id.uuidString)",
                title: entry.title,
                subtitle: WebAddress.host(of: entry.url),
                symbol: "clock",
                section: .history,
                kind: .history(entry.id)
            ))
        }
        return hits
    }

    private static func matchingTabs(_ query: String, workspace: Workspace) -> [CommandHit] {
        ranked(workspace.tabs.values, query: query) { tab in
            [tab.displayTitle, tab.url ?? ""]
        }
        .prefix(6)
        .map { tab in
            CommandHit(
                id: "tab-\(tab.id.uuidString)",
                title: tab.displayTitle,
                subtitle: WebAddress.host(of: tab.url),
                symbol: "square.on.square",
                section: .tabs,
                kind: .tab(tab.id)
            )
        }
    }

    private static func matchingBookmarks(_ query: String, workspace: Workspace) -> [CommandHit] {
        ranked(workspace.bookmarks, query: query) { [$0.title, $0.url] }
            .prefix(6)
            .map { bookmark in
                CommandHit(
                    id: "bookmark-\(bookmark.id.uuidString)",
                    title: bookmark.title,
                    subtitle: WebAddress.host(of: bookmark.url),
                    symbol: "bookmark",
                    section: .bookmarks,
                    kind: .bookmark(bookmark.id)
                )
            }
    }

    private static func matchingHistory(_ query: String, workspace: Workspace) -> [CommandHit] {
        ranked(workspace.history, query: query) { [$0.title, $0.url] }
            .prefix(6)
            .map { entry in
                CommandHit(
                    id: "history-\(entry.id.uuidString)",
                    title: entry.title,
                    subtitle: WebAddress.host(of: entry.url),
                    symbol: "clock",
                    section: .history,
                    kind: .history(entry.id)
                )
            }
    }

    private static func ranked<T>(_ values: some Sequence<T>, query: String, fields: (T) -> [String]) -> [T] {
        let needle = query.lowercased()
        return values
            .compactMap { value -> (Int, T)? in
                let haystacks = fields(value).map { $0.lowercased() }
                guard haystacks.contains(where: { $0.contains(needle) }) else { return nil }
                let score = haystacks.contains(where: { $0.hasPrefix(needle) }) ? 0 : 1
                return (score, value)
            }
            .sorted { $0.0 < $1.0 }
            .map(\.1)
    }
}
