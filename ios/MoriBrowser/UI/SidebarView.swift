import SwiftUI

private enum SidebarRow: Identifiable {
    case favorites
    case pinnedHeader
    case tab(UUID, indent: Bool)
    case folder(UUID, name: String, symbol: String, expanded: Bool)
    case todayHeader
    case emptyToday
    case newTab

    var id: String {
        switch self {
        case .favorites: return "favorites"
        case .pinnedHeader: return "pinned"
        case .tab(let id, let indent): return "tab-\(id.uuidString)-\(indent)"
        case .folder(let id, _, _, _): return "folder-\(id.uuidString)"
        case .todayHeader: return "today"
        case .emptyToday: return "empty"
        case .newTab: return "new"
        }
    }
}

struct SidebarView: View {
    @Bindable var store: BrowserStore
    var embedded: Bool
    @Environment(\.moriPalette) private var palette
    @Environment(\.horizontalSizeClass) private var sizeClass

    var body: some View {
        VStack(spacing: 0) {
            header
            if sizeClass != .regular {
                searchRow
                    .padding(.top, 14)
            }
            tabList
        }
        .background(Color.clear)
        .alert("New Folder", isPresented: $store.creatingFolder) {
            TextField("Name", text: $store.folderName)
            Button("Cancel", role: .cancel) { store.folderName = "" }
            Button("Create", action: store.createFolder)
        }
        .confirmationDialog(
            "Close today's tabs?",
            isPresented: $store.confirmClearToday,
            titleVisibility: .visible
        ) {
            Button("Close Tabs", role: .destructive) { store.clearToday() }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("Pinned tabs and favorites stay open.")
        }
    }

    private var header: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Text("Tabs")
                .font(.system(size: 26, weight: .semibold))
                .lineLimit(1)
            Spacer(minLength: 0)
            overflowMenu
            Button(action: store.newTab) {
                Image(systemName: "plus")
                    .font(.system(size: 18, weight: .semibold))
                    .frame(width: 36, height: 36)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel("New Tab")
        }
        .padding(.horizontal, 20)
        .padding(.top, 8)
        .foregroundStyle(palette.foreground)
    }

    private var searchRow: some View {
        Button(action: store.beginNavigation) {
            HStack(spacing: 8) {
                Image(systemName: "magnifyingglass")
                    .font(.system(size: 14, weight: .medium))
                Text("Search or enter address")
                    .lineLimit(1)
                Spacer(minLength: 0)
            }
            .font(.system(size: 15))
            .foregroundStyle(palette.mutedForeground)
            .padding(.horizontal, 12)
            .frame(height: 40)
            .background(Color.primary.opacity(0.05), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        }
        .buttonStyle(.plain)
        .padding(.horizontal, 16)
        .accessibilityIdentifier("addressPrompt")
    }

    private var overflowMenu: some View {
        Menu {
            Button("New Tab", action: store.newTab)
            Button("New Folder") { store.creatingFolder = true }
            Divider()
            Button("Library") { store.open(.library, embedded: embedded) }
            Button("Settings") { store.open(.settings, embedded: embedded) }
        } label: {
            Image(systemName: "ellipsis")
                .font(.system(size: 16, weight: .semibold))
                .frame(width: 32, height: 32)
                .contentShape(Rectangle())
        }
        .accessibilityLabel("Space menu")
    }

    private var tabList: some View {
        List {
            ForEach(rows) { row in
                rowView(row)
                    .listRowInsets(EdgeInsets(top: 1, leading: 12, bottom: 1, trailing: 12))
                    .listRowSeparator(.hidden)
                    .listRowBackground(Color.clear)
            }
        }
        .listStyle(.plain)
        .listRowSpacing(1)
        .scrollContentBackground(.hidden)
        .environment(\.defaultMinListRowHeight, 8)
    }

    private var rows: [SidebarRow] {
        var rows: [SidebarRow] = []
        if !store.workspace.favorites.isEmpty {
            rows.append(.favorites)
        }
        let space = store.activeSpace
        if !space.pinned.isEmpty {
            rows.append(.pinnedHeader)
        }
        for node in space.pinned {
            switch node {
            case .tab(let id):
                rows.append(.tab(id, indent: false))
            case .folder(let folder):
                rows.append(.folder(folder.id, name: folder.name, symbol: folder.symbol, expanded: folder.isExpanded))
                if folder.isExpanded {
                    rows.append(contentsOf: folder.tabIDs.map { .tab($0, indent: true) })
                }
            }
        }
        rows.append(.todayHeader)
        if space.today.isEmpty {
            rows.append(.emptyToday)
        } else {
            rows.append(contentsOf: space.today.map { .tab($0, indent: false) })
        }
        rows.append(.newTab)
        return rows
    }

    @ViewBuilder
    private func rowView(_ row: SidebarRow) -> some View {
        switch row {
        case .favorites:
            FavoriteGrid(store: store, large: false)
                .padding(.vertical, 8)
        case .pinnedHeader:
            sectionLabel("Pinned")
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.top, 8)
                .padding(.horizontal, 8)
        case .tab(let id, let indent):
            if let tab = store.workspace.tabs[id] {
                TabRow(store: store, tab: tab, indented: indent)
            }
        case .folder(let id, let name, let symbol, let expanded):
            Button { store.toggleFolder(id) } label: {
                HStack(spacing: 8) {
                    Image(systemName: expanded ? "chevron.down" : "chevron.right")
                        .font(.system(size: 11, weight: .semibold))
                        .frame(width: 12)
                        .foregroundStyle(palette.mutedForeground)
                    SymbolIcon(name: symbol, size: 14)
                    Text(name)
                        .font(.system(size: 15, weight: .semibold))
                        .lineLimit(1)
                    Spacer()
                }
                .foregroundStyle(palette.foreground)
                .frame(height: 36)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .contextMenu {
                Button("Delete Folder", role: .destructive) { store.deleteFolder(id) }
            }
        case .todayHeader:
            HStack {
                sectionLabel("Today")
                Spacer()
                if !store.activeSpace.today.isEmpty {
                    Button("Clear") { store.confirmClearToday = true }
                        .font(.system(size: 13, weight: .medium))
                        .foregroundStyle(palette.mutedForeground)
                }
            }
            .padding(.top, 14)
            .padding(.horizontal, 8)
        case .emptyToday:
            Text("No tabs yet")
                .font(.system(size: 15))
                .foregroundStyle(palette.mutedForeground)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 8)
                .padding(.vertical, 6)
        case .newTab:
            Button(action: store.newTab) {
                Label("New Tab", systemImage: "plus")
                    .font(.system(size: 15, weight: .medium))
                    .foregroundStyle(palette.mutedForeground)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .frame(height: 40)
                    .padding(.horizontal, 8)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
    }

    private func sectionLabel(_ text: String) -> some View {
        Text(text)
            .font(.system(size: 13, weight: .medium))
            .foregroundStyle(palette.mutedForeground)
    }
}

struct FavoriteGrid: View {
    var store: BrowserStore
    var large: Bool
    @Environment(\.moriPalette) private var palette

    private var columns: [GridItem] {
        Array(repeating: GridItem(.flexible(), spacing: large ? 16 : 10), count: 4)
    }

    var body: some View {
        LazyVGrid(columns: columns, spacing: large ? 18 : 12) {
            ForEach(store.workspace.favorites, id: \.self) { id in
                if let tab = store.workspace.tabs[id] {
                    let selected = store.selectedTabID == id
                    Button { store.select(tab: id) } label: {
                        VStack(spacing: 6) {
                            FaviconView(urlString: tab.url, title: tab.displayTitle, size: large ? 48 : 36)
                                .overlay {
                                    if selected {
                                        RoundedRectangle(cornerRadius: large ? 14 : 11, style: .continuous)
                                            .strokeBorder(palette.primary, lineWidth: 2)
                                            .padding(large ? -4 : -3)
                                    }
                                }
                            Text(tab.displayTitle)
                                .font(.system(size: large ? 12 : 11))
                                .foregroundStyle(palette.foreground.opacity(0.85))
                                .lineLimit(1)
                                .multilineTextAlignment(.center)
                        }
                        .frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(tab.displayTitle)
                    .contextMenu {
                        Button("Remove from Favorites") { store.toggleFavorite(tab: id) }
                        Button("Close", role: .destructive) { store.close(tab: id) }
                    }
                }
            }
        }
    }
}

struct TabRow: View {
    var store: BrowserStore
    var tab: TabRecord
    var indented: Bool
    @Environment(\.moriPalette) private var palette

    private var selected: Bool { store.selectedTabID == tab.id }
    private var isPrivate: Bool { store.workspace.space(containing: tab.id)?.isPrivate == true }

    var body: some View {
        HStack(spacing: 0) {
            Button { store.select(tab: tab.id) } label: {
                HStack(spacing: 10) {
                    FaviconView(urlString: tab.url, title: tab.displayTitle, isPrivate: isPrivate, size: 16)
                    Text(tab.displayTitle)
                        .font(.system(size: 15, weight: selected ? .medium : .regular))
                        .foregroundStyle(palette.foreground)
                        .lineLimit(1)
                    Spacer(minLength: 0)
                    if store.sessions[tab.id]?.isLoading == true {
                        ProgressView().controlSize(.small)
                    }
                }
                .padding(.leading, indented ? 28 : 10)
                .padding(.trailing, selected ? 4 : 10)
                .frame(height: 40)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            if selected {
                Button { store.close(tab: tab.id) } label: {
                    Image(systemName: "xmark")
                        .font(.system(size: 11, weight: .bold))
                        .foregroundStyle(palette.mutedForeground)
                        .frame(width: 28, height: 40)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Close \(tab.displayTitle)")
            }
        }
        .padding(.trailing, selected ? 4 : 0)
        .background(
            RoundedRectangle(cornerRadius: 10, style: .continuous)
                .fill(selected ? Color.primary.opacity(0.07) : Color.clear)
        )
        .accessibilityAddTraits(selected ? .isSelected : [])
        .swipeActions(edge: .trailing, allowsFullSwipe: true) {
            Button("Close", role: .destructive) { store.close(tab: tab.id) }
        }
        .contextMenu { tabMenu }
    }

    @ViewBuilder
    private var tabMenu: some View {
        if !isPrivate {
            Button(store.workspace.favorites.contains(tab.id) ? "Remove from Favorites" : "Add to Favorites") {
                store.toggleFavorite(tab: tab.id)
            }
            Button(store.workspace.isPinned(tab.id) ? "Unpin" : "Pin") {
                store.togglePin(tab: tab.id)
            }
            if !folders.isEmpty {
                Menu("Move to Folder") {
                    ForEach(folders) { folder in
                        Button(folder.name) { store.move(tab: tab.id, toFolder: folder.id) }
                    }
                }
            }
        }
        Button("Close", role: .destructive) { store.close(tab: tab.id) }
    }

    private var folders: [Folder] {
        store.activeSpace.pinned.compactMap { node in
            if case .folder(let folder) = node { return folder }
            return nil
        }
    }
}
