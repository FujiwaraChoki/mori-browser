import Foundation

struct Workspace: Equatable, Sendable {
    static let maxFavorites = 12
    static let maxHistory = 400

    var spaces: [Space]
    var activeSpaceID: UUID
    var favorites: [UUID]
    var tabs: [UUID: TabRecord]
    var bookmarks: [Bookmark]
    var history: [HistoryEntry]
    var settings: Settings
    var hasOnboarded: Bool

    var activeSpace: Space {
        spaces.first { $0.id == activeSpaceID } ?? spaces[0]
    }

    static func fresh() -> Workspace {
        let space = Space(name: "Personal", symbol: "circle")
        let tab = TabRecord.empty()
        var workspace = Workspace(
            spaces: [space],
            activeSpaceID: space.id,
            favorites: [],
            tabs: [tab.id: tab],
            bookmarks: [],
            history: [],
            settings: Settings(),
            hasOnboarded: false
        )
        workspace.spaces[0].today = [tab.id]
        workspace.spaces[0].selectedTabID = tab.id
        return workspace
    }

    static func sample() -> Workspace {
        var workspace = fresh()
        workspace.hasOnboarded = true
        if let index = workspace.index(of: workspace.activeSpaceID) {
            workspace.spaces[index].theme = ThemePreset.all.first { $0.id == "forest" }?.theme
        }
        let example = TabRecord(id: UUID(), title: "Example", url: "https://example.com", pinnedURL: nil)
        let notes = TabRecord(id: UUID(), title: "Notes", url: "https://example.com/notes", pinnedURL: "https://example.com/notes")
        let reading = TabRecord(id: UUID(), title: "Reading", url: "https://example.com/reading", pinnedURL: nil)
        workspace.tabs[example.id] = example
        workspace.tabs[notes.id] = notes
        workspace.tabs[reading.id] = reading
        workspace.favorites = [example.id]
        if let index = workspace.index(of: workspace.activeSpaceID) {
            workspace.spaces[index].pinned = [.tab(notes.id), .folder(Folder(name: "Later", tabIDs: [reading.id]))]
        }
        _ = workspace.addSpace(private: false)
        if let work = workspace.spaces.last {
            workspace.rename(space: work.id, to: "Work")
        }
        workspace.switchTo(workspace.spaces[0].id)
        workspace.bookmarks = [
            Bookmark(id: UUID(), title: "Example", url: "https://example.com", createdAt: Date()),
        ]
        workspace.history = [
            HistoryEntry(id: UUID(), title: "Example", url: "https://example.com", visitedAt: Date()),
        ]
        return workspace
    }

    func placement(of id: UUID) -> TabPlacement? {
        if favorites.contains(id) {
            return TabPlacement(spaceID: nil, kind: .favorite)
        }
        for space in spaces {
            if space.today.contains(id) {
                return TabPlacement(spaceID: space.id, kind: .today)
            }
            for node in space.pinned {
                switch node {
                case .tab(let tab) where tab == id:
                    return TabPlacement(spaceID: space.id, kind: .pinned)
                case .folder(let folder) where folder.tabIDs.contains(id):
                    return TabPlacement(spaceID: space.id, kind: .folder(folder.id))
                default:
                    break
                }
            }
        }
        return nil
    }

    func isPinned(_ id: UUID) -> Bool {
        if case .pinned = placement(of: id)?.kind { return true }
        return false
    }

    func space(containing id: UUID) -> Space? {
        guard let spaceID = placement(of: id)?.spaceID else { return nil }
        return spaces.first { $0.id == spaceID }
    }

    func index(of spaceID: UUID) -> Int? {
        spaces.firstIndex { $0.id == spaceID }
    }

    @discardableResult
    mutating func newTab(in spaceID: UUID) -> UUID {
        guard let index = index(of: spaceID) else {
            guard spaceID != activeSpaceID else { return UUID() }
            return newTab(in: activeSpaceID)
        }
        let tab = TabRecord.empty()
        tabs[tab.id] = tab
        spaces[index].today.append(tab.id)
        spaces[index].selectedTabID = tab.id
        activeSpaceID = spaceID
        return tab.id
    }

    mutating func navigate(tab id: UUID, to url: URL) {
        guard var tab = tabs[id] else { return }
        guard let scheme = url.scheme?.lowercased(), scheme == "http" || scheme == "https" else { return }
        tab.url = url.absoluteString
        if tab.title == "New Tab" || tab.title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            tab.title = url.host ?? url.absoluteString
        }
        if isPinned(id), tab.pinnedURL == nil {
            tab.pinnedURL = url.absoluteString
        }
        tabs[id] = tab
    }

    mutating func noteVisit(tab id: UUID, url: URL, title: String?, isPrivate: Bool, recordsHistory: Bool) {
        guard var tab = tabs[id] else { return }
        guard let scheme = url.scheme?.lowercased(), scheme == "http" || scheme == "https" else { return }
        tab.url = url.absoluteString
        if let title {
            let trimmed = title.trimmingCharacters(in: .whitespacesAndNewlines)
            if !trimmed.isEmpty { tab.title = trimmed }
        }
        if isPinned(id), tab.pinnedURL == nil {
            tab.pinnedURL = url.absoluteString
        }
        tabs[id] = tab
        guard !isPrivate else { return }
        let absolute = tab.url ?? url.absoluteString
        if recordsHistory {
            recordHistory(title: tab.displayTitle, url: absolute)
        } else if history.first?.url == absolute {
            history[0].title = tab.displayTitle
        }
    }

    mutating func select(tab id: UUID) {
        guard tabs[id] != nil else { return }
        if favorites.contains(id) {
            if let index = index(of: activeSpaceID) {
                spaces[index].selectedTabID = id
            }
            return
        }
        guard let spaceID = placement(of: id)?.spaceID, let index = index(of: spaceID) else { return }
        activeSpaceID = spaceID
        spaces[index].selectedTabID = id
    }

    mutating func switchTo(_ id: UUID) {
        guard spaces.contains(where: { $0.id == id }) else { return }
        activeSpaceID = id
    }

    @discardableResult
    mutating func close(tab id: UUID) -> ClosedTab? {
        guard let record = tabs[id] else { return nil }
        let place = placement(of: id)
        let successor = neighbor(of: id)
        remove(id)
        tabs[id] = nil
        for index in spaces.indices where spaces[index].selectedTabID == id {
            spaces[index].selectedTabID = successor
        }
        repairAllSelections()
        return ClosedTab(record: record, spaceID: place?.spaceID, kind: place?.kind ?? .today)
    }

    mutating func reopen(_ closed: ClosedTab) {
        tabs[closed.record.id] = closed.record
        switch closed.kind {
        case .favorite:
            if favorites.count < Self.maxFavorites {
                favorites.append(closed.record.id)
                if let index = index(of: activeSpaceID) {
                    spaces[index].selectedTabID = closed.record.id
                }
                return
            }
            appendToday(closed.record.id, spaceID: activeSpaceID)
        case .pinned:
            let spaceID = resolvedSpace(closed.spaceID)
            guard let index = index(of: spaceID) else { return }
            spaces[index].pinned.insert(.tab(closed.record.id), at: 0)
            spaces[index].selectedTabID = closed.record.id
            activeSpaceID = spaceID
        case .today:
            appendToday(closed.record.id, spaceID: resolvedSpace(closed.spaceID))
        case .folder(let folderID):
            if move(tab: closed.record.id, toFolder: folderID) { return }
            appendToday(closed.record.id, spaceID: resolvedSpace(closed.spaceID))
        }
    }

    @discardableResult
    mutating func favorite(tab id: UUID) -> Bool {
        guard tabs[id] != nil else { return false }
        if favorites.contains(id) { return true }
        guard favorites.count < Self.maxFavorites else { return false }
        remove(id)
        favorites.append(id)
        if let index = index(of: activeSpaceID) {
            spaces[index].selectedTabID = id
        }
        return true
    }

    mutating func unfavorite(tab id: UUID) {
        guard favorites.contains(id) else { return }
        favorites.removeAll { $0 == id }
        appendToday(id, spaceID: activeSpaceID)
    }

    mutating func pin(tab id: UUID) {
        guard var record = tabs[id] else { return }
        if isPinned(id) { return }
        let spaceID = placement(of: id)?.spaceID ?? activeSpaceID
        guard let index = index(of: spaceID) else { return }
        remove(id)
        if record.pinnedURL == nil { record.pinnedURL = record.url }
        tabs[id] = record
        spaces[index].pinned.insert(.tab(id), at: 0)
        spaces[index].selectedTabID = id
        activeSpaceID = spaceID
    }

    mutating func unpin(tab id: UUID) {
        guard isPinned(id), let spaceID = placement(of: id)?.spaceID else { return }
        remove(id)
        appendToday(id, spaceID: spaceID)
    }

    @discardableResult
    mutating func move(tab id: UUID, toFolder folderID: UUID) -> Bool {
        guard tabs[id] != nil, folderLocation(folderID) != nil else { return false }
        remove(id)
        guard let location = folderLocation(folderID) else { return false }
        guard case .folder(var folder) = spaces[location.space].pinned[location.node] else { return false }
        folder.tabIDs.append(id)
        folder.isExpanded = true
        spaces[location.space].pinned[location.node] = .folder(folder)
        spaces[location.space].selectedTabID = id
        activeSpaceID = spaces[location.space].id
        return true
    }

    @discardableResult
    mutating func addFolder(named name: String, in spaceID: UUID) -> UUID? {
        guard let index = index(of: spaceID) else { return nil }
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        let folder = Folder(name: trimmed.isEmpty ? "Folder" : String(trimmed.prefix(40)))
        spaces[index].pinned.append(.folder(folder))
        return folder.id
    }

    mutating func toggleFolder(_ id: UUID) {
        editFolder(id) { $0.isExpanded.toggle() }
    }

    mutating func renameFolder(_ id: UUID, to name: String) {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        editFolder(id) { $0.name = String(trimmed.prefix(40)) }
    }

    mutating func deleteFolder(_ id: UUID) {
        guard let location = folderLocation(id) else { return }
        guard case .folder(let folder) = spaces[location.space].pinned[location.node] else { return }
        let spaceID = spaces[location.space].id
        let tabIDs = folder.tabIDs
        spaces[location.space].pinned.remove(at: location.node)
        for tab in tabIDs {
            spaces[location.space].today.append(tab)
        }
        if spaces[location.space].selectedTabID == nil {
            spaces[location.space].selectedTabID = tabIDs.last
        }
        activeSpaceID = spaceID
    }

    @discardableResult
    mutating func clearToday(in spaceID: UUID) -> [ClosedTab] {
        guard let index = index(of: spaceID) else { return [] }
        let ids = spaces[index].today
        var closed: [ClosedTab] = []
        for id in ids {
            if let item = close(tab: id) {
                closed.append(item)
            }
        }
        return closed
    }

    @discardableResult
    mutating func addSpace(private isPrivate: Bool) -> UUID {
        let normalCount = spaces.filter { !$0.isPrivate }.count
        let privateCount = spaces.filter(\.isPrivate).count
        let theme: GradientTheme? = isPrivate ? SpaceThemes.privateSpace : SpaceThemes.pair(max(0, normalCount - 1))
        let symbol = isPrivate ? "lock" : SpaceSymbols.next(index: max(0, normalCount - 1))
        let name: String
        if isPrivate {
            name = privateCount == 0 ? "Private" : "Private \(privateCount + 1)"
        } else {
            name = "Space \(spaces.count + 1)"
        }
        var space = Space(name: name, symbol: symbol, theme: theme, isPrivate: isPrivate)
        let tab = TabRecord.empty()
        tabs[tab.id] = tab
        space.today = [tab.id]
        space.selectedTabID = tab.id
        spaces.append(space)
        activeSpaceID = space.id
        return space.id
    }

    @discardableResult
    mutating func deleteSpace(_ id: UUID) -> Bool {
        guard let index = index(of: id) else { return false }
        let space = spaces[index]
        if !space.isPrivate, spaces.filter({ !$0.isPrivate }).count <= 1 {
            return false
        }
        let owned = space.tabIDs
        spaces.remove(at: index)
        for tab in owned where placement(of: tab) == nil && !favorites.contains(tab) {
            tabs[tab] = nil
        }
        if activeSpaceID == id {
            activeSpaceID = spaces.first { !$0.isPrivate }?.id ?? spaces[0].id
        }
        repairAllSelections()
        return true
    }

    mutating func rename(space id: UUID, to name: String) {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, let index = index(of: id) else { return }
        spaces[index].name = String(trimmed.prefix(40))
    }

    mutating func setSymbol(_ symbol: String, for spaceID: UUID) {
        guard let index = index(of: spaceID), !symbol.isEmpty else { return }
        spaces[index].symbol = symbol
    }

    mutating func setTheme(_ theme: GradientTheme?, for spaceID: UUID) {
        guard let index = index(of: spaceID) else { return }
        spaces[index].theme = theme
    }

    @discardableResult
    mutating func toggleBookmark(title: String, url: String) -> Bool {
        if let index = bookmarks.firstIndex(where: { $0.url == url }) {
            bookmarks.remove(at: index)
            return false
        }
        bookmarks.insert(Bookmark(id: UUID(), title: title, url: url, createdAt: Date()), at: 0)
        return true
    }

    mutating func removeBookmark(_ id: UUID) {
        bookmarks.removeAll { $0.id == id }
    }

    mutating func removeHistory(_ id: UUID) {
        history.removeAll { $0.id == id }
    }

    mutating func clearHistory() {
        history = []
    }

    mutating func recordHistory(title: String, url: String) {
        guard let scheme = URL(string: url)?.scheme?.lowercased(), scheme == "http" || scheme == "https" else { return }
        if history.first?.url == url {
            history[0].title = title
            history[0].visitedAt = Date()
            return
        }
        history.removeAll { $0.url == url }
        history.insert(HistoryEntry(id: UUID(), title: title, url: url, visitedAt: Date()), at: 0)
        if history.count > Self.maxHistory {
            history.removeLast(history.count - Self.maxHistory)
        }
    }

    func snapshot() -> Snapshot {
        var copy = self
        copy.spaces.removeAll { $0.isPrivate }
        if copy.activeSpaceID != activeSpaceID || !copy.spaces.contains(where: { $0.id == copy.activeSpaceID }) {
            copy.activeSpaceID = copy.spaces.first { !$0.isPrivate }?.id ?? copy.activeSpaceID
        }
        copy.sanitize()
        return Snapshot(workspace: copy)
    }

    static func restored(_ snapshot: Snapshot) -> Workspace {
        var workspace = Workspace(
            spaces: snapshot.spaces,
            activeSpaceID: snapshot.activeSpaceID,
            favorites: snapshot.favorites,
            tabs: Dictionary(uniqueKeysWithValues: snapshot.tabs.map { ($0.id, $0) }),
            bookmarks: snapshot.bookmarks,
            history: snapshot.history,
            settings: snapshot.settings,
            hasOnboarded: snapshot.hasOnboarded
        )
        workspace.sanitize()
        return workspace
    }

    mutating func sanitize() {
        favorites = favorites.filter { tabs[$0] != nil }
        spaces = spaces.map { space in
            var space = space
            space.today = space.today.filter { tabs[$0] != nil }
            space.pinned = space.pinned.compactMap { node in
                switch node {
                case .tab(let id):
                    return tabs[id] == nil ? nil : .tab(id)
                case .folder(var folder):
                    folder.tabIDs = folder.tabIDs.filter { tabs[$0] != nil }
                    return .folder(folder)
                }
            }
            if let selected = space.selectedTabID, tabs[selected] == nil {
                space.selectedTabID = nil
            }
            return space
        }
        if !spaces.contains(where: { !$0.isPrivate }) {
            spaces.insert(Space(name: "Personal", symbol: "circle"), at: 0)
        }
        if !spaces.contains(where: { $0.id == activeSpaceID }) {
            activeSpaceID = spaces.first { !$0.isPrivate }?.id ?? spaces[0].id
        }
        repairAllSelections()
        let referenced = referencedTabIDs()
        tabs = tabs.filter { referenced.contains($0.key) }
        bookmarks.removeAll { URL(string: $0.url) == nil }
        if history.count > Self.maxHistory {
            history = Array(history.prefix(Self.maxHistory))
        }
    }

    private func referencedTabIDs() -> Set<UUID> {
        var ids = Set(favorites)
        for space in spaces {
            ids.formUnion(space.tabIDs)
            if let selected = space.selectedTabID { ids.insert(selected) }
        }
        return ids
    }

    private mutating func repairAllSelections() {
        for index in spaces.indices {
            let space = spaces[index]
            if let selected = space.selectedTabID,
               tabs[selected] != nil,
               favorites.contains(selected) || space.contains(selected) {
                continue
            }
            if let fallback = space.today.last ?? space.tabIDs.last ?? favorites.first, tabs[fallback] != nil {
                spaces[index].selectedTabID = fallback
                continue
            }
            let tab = TabRecord.empty()
            tabs[tab.id] = tab
            spaces[index].today.append(tab.id)
            spaces[index].selectedTabID = tab.id
        }
    }

    private func neighbor(of id: UUID) -> UUID? {
        if let index = favorites.firstIndex(of: id) {
            if index > 0 { return favorites[index - 1] }
            return favorites.dropFirst().first
        }
        for space in spaces {
            if let index = space.today.firstIndex(of: id) {
                if index > 0 { return space.today[index - 1] }
                return space.today.dropFirst().first ?? space.tabIDs.first { $0 != id }
            }
        }
        return nil
    }

    private mutating func remove(_ id: UUID) {
        favorites.removeAll { $0 == id }
        for index in spaces.indices {
            spaces[index].today.removeAll { $0 == id }
            spaces[index].pinned = spaces[index].pinned.compactMap { node in
                switch node {
                case .tab(let tab) where tab == id:
                    return nil
                case .folder(var folder):
                    folder.tabIDs.removeAll { $0 == id }
                    return .folder(folder)
                default:
                    return node
                }
            }
        }
    }

    private mutating func appendToday(_ id: UUID, spaceID: UUID) {
        guard tabs[id] != nil, let index = index(of: spaceID) else { return }
        spaces[index].today.append(id)
        spaces[index].selectedTabID = id
        activeSpaceID = spaceID
    }

    private func resolvedSpace(_ id: UUID?) -> UUID {
        if let id, spaces.contains(where: { $0.id == id }) { return id }
        return activeSpaceID
    }

    private func folderLocation(_ id: UUID) -> (space: Int, node: Int)? {
        for space in spaces.indices {
            for node in spaces[space].pinned.indices {
                if case .folder(let folder) = spaces[space].pinned[node], folder.id == id {
                    return (space, node)
                }
            }
        }
        return nil
    }

    private mutating func editFolder(_ id: UUID, _ body: (inout Folder) -> Void) {
        guard let location = folderLocation(id) else { return }
        guard case .folder(var folder) = spaces[location.space].pinned[location.node] else { return }
        body(&folder)
        spaces[location.space].pinned[location.node] = .folder(folder)
    }
}

struct Snapshot: Codable, Equatable {
    var version: Int
    var spaces: [Space]
    var activeSpaceID: UUID
    var favorites: [UUID]
    var tabs: [TabRecord]
    var bookmarks: [Bookmark]
    var history: [HistoryEntry]
    var settings: Settings
    var hasOnboarded: Bool

    init(workspace: Workspace) {
        version = 1
        spaces = workspace.spaces
        activeSpaceID = workspace.activeSpaceID
        favorites = workspace.favorites
        tabs = workspace.tabs.values.sorted { $0.id.uuidString < $1.id.uuidString }
        bookmarks = workspace.bookmarks
        history = workspace.history
        settings = workspace.settings
        hasOnboarded = workspace.hasOnboarded
    }
}

enum SpaceThemes {
    static let pairs: [(UInt32, UInt32)] = [
        (0x6E5BD8, 0x9B5BD8),
        (0x3577D4, 0x5B5BD8),
        (0x1F9488, 0x2F78A8),
        (0xD9702E, 0xC94E5A),
        (0xD24E7C, 0x9C4ED2),
        (0x4F9A3C, 0x2C8C78),
        (0xC48A1E, 0xC45A1E),
        (0x8E5BD0, 0xC85BA8),
    ]

    static let privateSpace = GradientTheme(
        colors: [0x3D3A4A, 0x23232A],
        opacity: 0.72,
        scheme: .dark,
        presetID: nil
    )

    static func pair(_ index: Int) -> GradientTheme {
        let colors = pairs[index % pairs.count]
        return GradientTheme(colors: [colors.0, colors.1], opacity: 0.55, scheme: .dark, presetID: nil)
    }
}
