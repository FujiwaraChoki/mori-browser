import Foundation

struct TabRecord: Codable, Equatable, Identifiable, Sendable {
    var id: UUID
    var title: String
    var url: String?
    /// Address remembered when the tab was pinned.
    var pinnedURL: String?

    static func empty() -> TabRecord {
        TabRecord(id: UUID(), title: "New Tab", url: nil, pinnedURL: nil)
    }

    var displayTitle: String {
        let trimmed = title.trimmingCharacters(in: .whitespacesAndNewlines)
        if !trimmed.isEmpty, trimmed != "New Tab" { return trimmed }
        if let host = WebAddress.host(of: url) { return host }
        return "New Tab"
    }

    var resolvedURL: URL? {
        if let url, let parsed = URL(string: url) { return parsed }
        if let pinnedURL, let parsed = URL(string: pinnedURL) { return parsed }
        return nil
    }
}

struct Folder: Codable, Equatable, Identifiable, Sendable {
    var id: UUID
    var name: String
    var symbol: String
    var tabIDs: [UUID]
    var isExpanded: Bool

    init(
        id: UUID = UUID(),
        name: String,
        symbol: String = "folder",
        tabIDs: [UUID] = [],
        isExpanded: Bool = true
    ) {
        self.id = id
        self.name = name
        self.symbol = symbol
        self.tabIDs = tabIDs
        self.isExpanded = isExpanded
    }
}

enum SidebarNode: Codable, Equatable, Identifiable, Sendable {
    case tab(UUID)
    case folder(Folder)

    var id: UUID {
        switch self {
        case .tab(let id): return id
        case .folder(let folder): return folder.id
        }
    }
}

struct GradientTheme: Codable, Equatable, Sendable {
    enum ForcedScheme: String, Codable, Equatable, Sendable {
        case light
        case dark
    }

    var colors: [UInt32]
    var opacity: Double
    var scheme: ForcedScheme?
    var presetID: String?
}

struct Space: Codable, Equatable, Identifiable, Sendable {
    var id: UUID
    var name: String
    var symbol: String
    var theme: GradientTheme?
    var isPrivate: Bool
    var pinned: [SidebarNode]
    var today: [UUID]
    var selectedTabID: UUID?

    init(
        id: UUID = UUID(),
        name: String,
        symbol: String = "circle",
        theme: GradientTheme? = nil,
        isPrivate: Bool = false,
        pinned: [SidebarNode] = [],
        today: [UUID] = [],
        selectedTabID: UUID? = nil
    ) {
        self.id = id
        self.name = name
        self.symbol = symbol
        self.theme = theme
        self.isPrivate = isPrivate
        self.pinned = pinned
        self.today = today
        self.selectedTabID = selectedTabID
    }

    func contains(_ tab: UUID) -> Bool {
        if today.contains(tab) { return true }
        for node in pinned {
            switch node {
            case .tab(let id) where id == tab:
                return true
            case .folder(let folder) where folder.tabIDs.contains(tab):
                return true
            default:
                break
            }
        }
        return false
    }

    var tabIDs: [UUID] {
        var ids = today
        for node in pinned {
            switch node {
            case .tab(let id):
                ids.append(id)
            case .folder(let folder):
                ids.append(contentsOf: folder.tabIDs)
            }
        }
        return ids
    }
}

struct Bookmark: Codable, Equatable, Identifiable, Sendable {
    var id: UUID
    var title: String
    var url: String
    var createdAt: Date
}

struct HistoryEntry: Codable, Equatable, Identifiable, Sendable {
    var id: UUID
    var title: String
    var url: String
    var visitedAt: Date
}

enum Appearance: String, Codable, Equatable, Sendable, CaseIterable {
    case system
    case light
    case dark

    var label: String {
        switch self {
        case .system: return "System"
        case .light: return "Light"
        case .dark: return "Dark"
        }
    }
}

enum SearchEngine: String, Codable, Equatable, Sendable, CaseIterable {
    case google
    case duckDuckGo
    case bing
    case brave

    var label: String {
        switch self {
        case .google: return "Google"
        case .duckDuckGo: return "DuckDuckGo"
        case .bing: return "Bing"
        case .brave: return "Brave"
        }
    }

    func searchURL(for query: String) -> URL {
        var components = URLComponents(string: root)!
        var items = components.queryItems ?? []
        items.append(URLQueryItem(name: parameter, value: query))
        components.queryItems = items
        return components.url ?? URL(string: "https://www.google.com/search?q=")!
    }

    func suggestURL(for query: String) -> URL? {
        let trimmed = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        var components = URLComponents(string: suggestRoot)
        var items = components?.queryItems ?? []
        items.append(URLQueryItem(name: suggestParameter, value: trimmed))
        components?.queryItems = items
        return components?.url
    }

    private var root: String {
        switch self {
        case .google: return "https://www.google.com/search"
        case .duckDuckGo: return "https://duckduckgo.com/"
        case .bing: return "https://www.bing.com/search"
        case .brave: return "https://search.brave.com/search"
        }
    }

    private var parameter: String { "q" }

    private var suggestRoot: String {
        switch self {
        case .google:
            return "https://suggestqueries.google.com/complete/search?client=firefox"
        case .duckDuckGo:
            return "https://duckduckgo.com/ac/?type=list"
        case .bing:
            return "https://api.bing.com/osjson.aspx"
        case .brave:
            return "https://search.brave.com/api/suggest"
        }
    }

    private var suggestParameter: String {
        self == .bing ? "query" : "q"
    }
}

struct Settings: Codable, Equatable, Sendable {
    var appearance: Appearance
    var searchEngine: SearchEngine
    var zoomByHost: [String: Double]

    init(
        appearance: Appearance = .system,
        searchEngine: SearchEngine = .google,
        zoomByHost: [String: Double] = [:]
    ) {
        self.appearance = appearance
        self.searchEngine = searchEngine
        self.zoomByHost = zoomByHost
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        appearance = try container.decodeIfPresent(Appearance.self, forKey: .appearance) ?? .system
        searchEngine = try container.decodeIfPresent(SearchEngine.self, forKey: .searchEngine) ?? .google
        zoomByHost = try container.decodeIfPresent([String: Double].self, forKey: .zoomByHost) ?? [:]
    }
}

struct TabPlacement: Equatable, Sendable {
    enum Kind: Equatable, Sendable {
        case favorite
        case pinned
        case today
        case folder(UUID)
    }

    var spaceID: UUID?
    var kind: Kind
}

struct ClosedTab: Equatable, Sendable {
    var record: TabRecord
    var spaceID: UUID?
    var kind: TabPlacement.Kind
}

enum WebAddress {
    static func host(of urlString: String?) -> String? {
        guard let urlString, let url = URL(string: urlString) else { return nil }
        return url.host
    }

    static func monogramLetter(urlString: String?, title: String) -> String {
        if let host = host(of: urlString) {
            let stripped = host.lowercased().hasPrefix("www.") ? String(host.dropFirst(4)) : host
            if let first = stripped.first { return String(first).uppercased() }
        }
        let trimmed = title.trimmingCharacters(in: .whitespacesAndNewlines)
        if let first = trimmed.first, first != "N" || trimmed != "New Tab" {
            return String(first).uppercased()
        }
        return "•"
    }
}

enum SpaceSymbols {
    static let sequence = [
        "briefcase",
        "chevron.left.forwardslash.chevron.right",
        "book",
        "paintpalette",
        "music.note",
        "leaf",
        "star",
        "heart",
        "flame",
        "globe",
    ]

    static let choices = [
        "circle", "house", "briefcase", "chevron.left.forwardslash.chevron.right",
        "book", "paintpalette", "music.note", "leaf", "star", "heart", "flame",
        "globe", "moon", "sun.max", "graduationcap", "camera", "airplane", "bag",
        "sparkles", "folder", "cup.and.saucer", "lock",
    ]

    static let emoji = ["😀", "😎", "🤓", "🏠", "💼", "📚", "🎨", "🎵", "🎮", "☕️", "🌱", "🔥", "⭐️", "❤️", "🚀"]

    static func next(index: Int) -> String {
        sequence[index % sequence.count]
    }
}
