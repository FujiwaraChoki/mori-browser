import SwiftUI
import UniformTypeIdentifiers

/// The vertical sidebar — Arc/SigmaOS-inspired. Top-to-bottom:
/// a header carrying the browser controls (nav + omnibox + downloads), a
/// pinned-tab tile grid, collapsible folders, the loose (unfiled) tabs under a
/// New Tab row, and a bottom action bar. Translucent glass over the Mori
/// `--sidebar-*` tokens.
struct Sidebar: View {
    @ObservedObject var store: BrowserStore
    @ObservedObject private var settings = BrowserSettings.shared

    /// The tab currently being dragged in the sidebar, shared across all drop
    /// targets so any container can reorder/accept it live. Held here at the top
    /// level and threaded down as a binding.
    @State private var draggingTabID: BrowserTab.ID?

    /// Live width while the resize handle is being dragged. Kept local so each
    /// drag frame is a cheap in-view update — the persisted (and UserDefaults-
    /// backed) `settings.sidebarWidth` is only written once, on release.
    @State private var liveWidth: CGFloat?

    /// The web card floats with an 8pt gap on its sidebar-facing edge. Trim the
    /// row padding by that gap on the same side so tab cards sit evenly inset
    /// within the visible chrome instead of crowding the outer window edge.
    private static let webCardGap: CGFloat = 8
    private func rowInsets(_ base: CGFloat) -> EdgeInsets {
        let trimLeading = settings.sidebarPosition == .right
        return EdgeInsets(
            top: 0,
            leading: base - (trimLeading ? Self.webCardGap : 0),
            bottom: 0,
            trailing: base - (trimLeading ? 0 : Self.webCardGap)
        )
    }

    var body: some View {
        VStack(spacing: 0) {
            if let tab = store.selectedTab ?? store.tabs.first {
                SidebarHeader(store: store, tab: tab)
                    .zIndex(10)
            }
            ScrollView {
                VStack(alignment: .leading, spacing: 10) {
                    if !store.pinnedTabs.isEmpty || draggingTabID != nil {
                        PinnedGrid(store: store, draggingTabID: $draggingTabID)
                            .padding(rowInsets(10))
                    }

                    if !store.folders.isEmpty {
                        FolderSection(store: store, draggingTabID: $draggingTabID)
                            .padding(rowInsets(8))
                    }

                    NewTabRow { store.presentLauncher() }
                        .padding(rowInsets(8))
                        // Pull up to tighten the gap above New Tab in every
                        // state (first row, or below pins/folders).
                        .padding(.top, -6)
                        .onDrop(of: SidebarTabDrag.acceptedTypes,
                                delegate: TabReorderDropDelegate(
                                    target: .loose(index: 0),
                                    draggingID: $draggingTabID,
                                    store: store))

                    LooseTabList(store: store, draggingTabID: $draggingTabID)
                        .padding(rowInsets(8))
                        .padding(.bottom, 10)
                }
                .padding(.top, 8)
            }
            SidebarMediaSection(store: store, media: store.media)
            SidebarBottomBar(store: store)
        }
        .frame(width: liveWidth ?? settings.sidebarWidth)
        .contentShape(Rectangle())
        .contextMenu { SidebarContextMenu(store: store) }
        .onDrop(of: SidebarTabDrag.acceptedTypes,
                delegate: TabReorderDropDelegate(
                    target: .loose(index: store.looseTabs.count),
                    draggingID: $draggingTabID,
                    store: store,
                    moveOnEnter: false))
        // Resize handle on the inner (web-card-facing) edge: leading when the
        // sidebar sits on the right, trailing when it sits on the left.
        .overlay(alignment: settings.sidebarPosition == .right ? .leading : .trailing) {
            SidebarResizeHandle(store: store, position: settings.sidebarPosition,
                                liveWidth: $liveWidth)
        }
        // No own background: the unified chrome surface (set on the root) shows
        // through, so the sidebar and the card's inset gaps are the same color.
    }
}

/// A thin, draggable strip along the sidebar's inner edge that resizes it.
/// Shows a faint divider on hover (hidden while dragging) and a resize cursor.
/// During the drag it only updates the parent's cheap `liveWidth` state;
/// the persisted `settings.sidebarWidth` is written once, on release.
private struct SidebarResizeHandle: View {
    @ObservedObject var store: BrowserStore
    let position: SidebarPosition
    @Binding var liveWidth: CGFloat?
    @ObservedObject private var settings = BrowserSettings.shared
    @State private var dragStartWidth: CGFloat?

    private static let hitWidth: CGFloat = 8

    var body: some View {
        Rectangle()
            .fill(Color.clear)
            .frame(width: Self.hitWidth)
            .contentShape(Rectangle())
            .onHover { inside in
                if inside { NSCursor.resizeLeftRight.push() } else { NSCursor.pop() }
            }
            .gesture(
                DragGesture(minimumDistance: 1)
                    .onChanged { value in
                        let start = dragStartWidth ?? settings.sidebarWidth
                        if dragStartWidth == nil {
                            dragStartWidth = start
                            store.isResizingSidebar = true
                        }
                        // Right sidebar grows when dragged left (negative dx);
                        // left sidebar grows when dragged right (positive dx).
                        let delta = position == .right ? -value.translation.width
                                                       : value.translation.width
                        liveWidth = (start + delta).clamped(
                            to: BrowserSettings.minSidebarWidth...BrowserSettings.maxSidebarWidth)
                    }
                    .onEnded { _ in
                        if let final = liveWidth { settings.sidebarWidth = final }
                        dragStartWidth = nil
                        liveWidth = nil
                        // Unfreeze the web card; the CEF view resizes once now.
                        store.isResizingSidebar = false
                    }
            )
    }
}

/// General right-click menu for the sidebar background and non-row chrome.
private struct SidebarContextMenu: View {
    @ObservedObject var store: BrowserStore
    @ObservedObject private var settings = BrowserSettings.shared

    var body: some View {
        Button("New Tab") {
            store.newTab()
        }
        Button("Add New Folder") {
            store.addFolderForEditing()
        }

        Divider()

        Button(store.aiPanelVisible ? "Hide AI Panel" : "Show AI Panel") {
            store.toggleAIPanel()
        }
        Menu("Sidebar Side") {
            ForEach(SidebarPosition.allCases) { position in
                Button(position.label) {
                    settings.sidebarPosition = position
                }
            }
        }
        Button("Hide Sidebar") {
            store.toggleSidebar()
        }

        Divider()

        Button("Settings") {
            store.settingsVisible = true
        }
    }
}

/// Observes the media controller so the player strip appears only for playback
/// happening outside the current tab.
private struct SidebarMediaSection: View {
    @ObservedObject var store: BrowserStore
    @ObservedObject var media: MediaController

    var body: some View {
        if shouldShowMedia {
            MediaPlayerStrip(store: store, media: media)
                .transition(.move(edge: .bottom).combined(with: .opacity))
                .animation(Motion.reveal, value: shouldShowMedia)
        }
    }

    private var shouldShowMedia: Bool {
        guard media.hasMedia else { return false }
        guard let owningTab = media.resolveTab?(media.state.browserId) else {
            return true
        }
        return owningTab.id != store.selectedTabID
    }
}

// MARK: - Header (relocated browser chrome)

/// The sidebar's top section now hosts the browser controls that used to live in
/// the top toolbar: the sidebar toggle, back / forward / reload, and the
/// omnibox. The nav row carries the toggle on the left and the nav buttons on
/// the right, with the full-width address field below, à la Arc/Dia.
private struct SidebarHeader: View {
    @ObservedObject var store: BrowserStore
    @ObservedObject var tab: BrowserTab
    @ObservedObject private var settings = BrowserSettings.shared
    @ObservedObject private var downloads = DownloadStore.shared
    @State private var showDownloads = false

    var body: some View {
        VStack(spacing: 8) {
            HStack(spacing: 4) {
                IconButton(systemName: settings.sidebarPosition.symbol, size: 28) {
                    store.toggleSidebar()
                }
                    .help("Toggle sidebar")
                Spacer()
                IconButton(systemName: "arrow.backward", size: 28,
                           disabled: !tab.canGoBack) { store.goBack() }
                IconButton(systemName: "arrow.forward", size: 28,
                           disabled: !tab.canGoForward) { store.goForward() }
                IconButton(systemName: tab.isLoading ? "xmark" : "arrow.clockwise",
                           size: 28) {
                    tab.isLoading ? store.stop() : store.reload()
                }
                DownloadsButton(downloads: downloads, isOpen: $showDownloads)
            }

            Omnibox(store: store, tab: tab)
                .frame(maxWidth: .infinity)
        }
        // Mirror the tab rows: trim the padding on the web-card-facing edge by
        // its 8pt float gap so the header reads as evenly inset, not crowded
        // toward the outer window edge.
        .padding(.leading, settings.sidebarPosition == .right ? 2 : 10)
        .padding(.trailing, settings.sidebarPosition == .right ? 10 : 2)
        .padding(.top, 10)
        .padding(.bottom, 6)
    }
}

// MARK: - Pinned tiles

private struct PinnedGrid: View {
    @ObservedObject var store: BrowserStore
    @Binding var draggingTabID: BrowserTab.ID?
    @State private var dropTargeted = false

    private let columns = [GridItem(.adaptive(minimum: 64, maximum: 92), spacing: 6)]

    var body: some View {
        LazyVGrid(columns: columns, spacing: 6) {
            if store.pinnedTabs.isEmpty {
                SidebarDropCatchZone(height: 40,
                                     cornerRadius: TabSurface.radius,
                                     isTargeted: dropTargeted)
            }

            ForEach(Array(store.pinnedTabs.enumerated()), id: \.element.id) { idx, tab in
                PinnedTile(
                    tab: tab,
                    isSelected: tab.id == store.selectedTabID,
                    onSelect: { store.selectTab(tab.id) }
                )
                .contextMenu { TabMenu(store: store, tab: tab) }
                .onDrag {
                    draggingTabID = tab.id
                    return SidebarTabDrag.provider(for: tab.id)
                }
                .onDrop(of: SidebarTabDrag.acceptedTypes, delegate: TabReorderDropDelegate(
                    target: .pinned(index: idx),
                    draggingID: $draggingTabID,
                    store: store))
            }
        }
        // Catch-all: dropping anywhere in the grid appends to the pins.
        .frame(maxWidth: .infinity)
        .contentShape(Rectangle())
        .onDrop(of: SidebarTabDrag.acceptedTypes, delegate: TabReorderDropDelegate(
            target: .pinned(index: store.pinnedTabs.count),
            draggingID: $draggingTabID,
            store: store,
            isTargeted: $dropTargeted))
    }
}

private struct PinnedTile: View {
    @ObservedObject var tab: BrowserTab
    let isSelected: Bool
    let onSelect: () -> Void

    @Environment(\.palette) private var p
    @Environment(\.colorScheme) private var scheme
    @State private var hovering = false

    var body: some View {
        Favicon(icon: tab.faviconURL, page: tab.urlString, image: tab.faviconImage,
                isLoading: tab.isLoading, size: 24)
            .frame(height: 48)
            .frame(maxWidth: .infinity)
            .background(
                RoundedRectangle(cornerRadius: TabSurface.radius, style: .continuous)
                    .fill(tileFill)
                    .shadow(color: isSelected ? TabSurface.shadow(scheme) : .clear,
                            radius: isSelected ? TabSurface.shadowRadius : 0,
                            x: 0, y: isSelected ? TabSurface.shadowY : 0)
            )
            .contentShape(Rectangle())
            .pressShrink(perform: onSelect)
            .onHover { hovering = $0 }
            .help(tab.title)
    }

    private var tileFill: Color {
        if isSelected { return TabSurface.selectedFill(scheme) }
        if hovering { return TabSurface.hoverFill(scheme) }
        return TabSurface.tileRestFill(scheme)
    }
}

// MARK: - Folders

private struct FolderSection: View {
    @ObservedObject var store: BrowserStore
    @Binding var draggingTabID: BrowserTab.ID?

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            ForEach(store.folders) { folder in
                FolderRow(store: store, folder: folder, draggingTabID: $draggingTabID)
            }
        }
    }
}

private struct FolderRow: View {
    @ObservedObject var store: BrowserStore
    let folder: TabFolder
    @Binding var draggingTabID: BrowserTab.ID?

    @Environment(\.palette) private var p
    @State private var hovering = false
    @State private var headerDropTargeted = false
    @State private var appendDropTargeted = false
    @State private var isEditing = false
    @State private var draftName = ""
    @FocusState private var nameFocused: Bool

    private var childTabs: [BrowserTab] { store.tabs(in: folder) }

    private var containsActiveTab: Bool {
        childTabs.contains { $0.id == store.selectedTabID }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            // Folder header row.
            HStack(spacing: 8) {
                MorphingFolderIcon(
                    isOpen: folder.isExpanded,
                    showsDots: !folder.isExpanded && containsActiveTab,
                    symbol: folder.symbol,
                    size: 24,
                    frontColor: p.primary.color.opacity(0.18),
                    backColor: p.primary.color.opacity(0.32),
                    stroke: p.sidebarForeground.color.opacity(0.55),
                    glyphColor: p.sidebarForeground.color.opacity(0.85),
                    surface: p.sidebar.color
                )
                .frame(width: 24, height: 24)

                if isEditing {
                    TextField("Folder", text: $draftName)
                        .textFieldStyle(.plain)
                        .font(Typography.ui(Typography.base, weight: .medium))
                        .foregroundStyle(p.sidebarForeground.color)
                        .focused($nameFocused)
                        .onSubmit(commitRename)
                        .onChange(of: nameFocused) { _, focused in
                            if !focused { commitRename() }
                        }
                } else {
                    Text(folder.name)
                        .font(Typography.ui(Typography.base, weight: .medium))
                        .foregroundStyle(p.sidebarForeground.color)
                        .lineLimit(1)
                }

                Spacer(minLength: 4)
            }
            .padding(.horizontal, 9)
            .frame(height: 32)
            .background(
                RoundedRectangle(cornerRadius: Radius.lg, style: .continuous)
                    .fill((hovering || headerDropTargeted) ? p.foreground.color.opacity(0.05) : .clear)
            )
            .contentShape(Rectangle())
            .onTapGesture { if !isEditing { store.toggleFolder(folder.id) } }
            .onHover { hovering = $0 }
            .contextMenu {
                Button("Rename") { beginRename() }
                Button("New Tab in Folder") {
                    let tab = store.newTab()
                    store.addTab(tab.id, toFolder: folder.id)
                }
                Divider()
                Button("Delete Folder", role: .destructive) { store.deleteFolder(folder.id) }
            }
            // Dropping onto the header appends the tab and expands the folder.
            .onDrop(of: SidebarTabDrag.acceptedTypes, delegate: TabReorderDropDelegate(
                target: .folder(id: folder.id, index: Int.max),
                draggingID: $draggingTabID,
                store: store,
                isTargeted: $headerDropTargeted))

            // Nested tabs.
            if folder.isExpanded {
                ForEach(Array(childTabs.enumerated()), id: \.element.id) { idx, tab in
                    TabRow(
                        tab: tab,
                        isSelected: tab.id == store.selectedTabID,
                        onSelect: { store.selectTab(tab.id) },
                        onClose: { store.closeTab(tab.id) }
                    )
                    .padding(.leading, 16)
                    .transition(.tabClose)
                    .contextMenu { TabMenu(store: store, tab: tab) }
                    .onDrag {
                        draggingTabID = tab.id
                        return SidebarTabDrag.provider(for: tab.id)
                    }
                    .onDrop(of: SidebarTabDrag.acceptedTypes, delegate: TabReorderDropDelegate(
                        target: .folder(id: folder.id, index: idx),
                        draggingID: $draggingTabID,
                        store: store))
                }

                // End cap: supports empty folders and appending after the last row.
                SidebarDropCatchZone(height: childTabs.isEmpty ? 12 : 8,
                                     cornerRadius: Radius.sm,
                                     isTargeted: appendDropTargeted)
                    .padding(.leading, 16)
                    .onDrop(of: SidebarTabDrag.acceptedTypes,
                            delegate: TabReorderDropDelegate(
                                target: .folder(id: folder.id, index: Int.max),
                                draggingID: $draggingTabID,
                                store: store,
                                isTargeted: $appendDropTargeted))
            }
        }
        .onAppear(perform: beginRenameIfRequested)
        .onChange(of: store.folderIDPendingRename) { _, _ in
            beginRenameIfRequested()
        }
    }

    private func beginRenameIfRequested() {
        guard store.folderIDPendingRename == folder.id else { return }
        beginRename()
        store.consumeFolderRenameRequest(for: folder.id)
    }

    private func beginRename() {
        draftName = folder.name
        isEditing = true
        DispatchQueue.main.async { nameFocused = true }
    }

    private func commitRename() {
        guard isEditing else { return }
        let trimmed = draftName.trimmingCharacters(in: .whitespacesAndNewlines)
        store.renameFolder(folder.id, to: trimmed.isEmpty ? "Folder" : trimmed)
        isEditing = false
    }
}

// MARK: - Loose tabs

private struct LooseTabList: View {
    @ObservedObject var store: BrowserStore
    @Binding var draggingTabID: BrowserTab.ID?
    @State private var appendDropTargeted = false

    var body: some View {
        LazyVStack(spacing: 4) {
            ForEach(Array(store.looseTabs.enumerated()), id: \.element.id) { idx, tab in
                TabRow(
                    tab: tab,
                    isSelected: tab.id == store.selectedTabID,
                    onSelect: { store.selectTab(tab.id) },
                    onClose: { store.closeTab(tab.id) }
                )
                .transition(.tabClose)
                .contextMenu { TabMenu(store: store, tab: tab) }
                .onDrag {
                    draggingTabID = tab.id
                    return SidebarTabDrag.provider(for: tab.id)
                }
                .onDrop(of: SidebarTabDrag.acceptedTypes, delegate: TabReorderDropDelegate(
                    target: .loose(index: idx),
                    draggingID: $draggingTabID,
                    store: store))
            }

            // Catch zone: dropping in the empty area below the rows appends to
            // the loose list. Min height gives an always-present target even
            // when there are no loose tabs.
            SidebarDropCatchZone(height: 24,
                                 cornerRadius: Radius.sm,
                                 isTargeted: appendDropTargeted)
                .onDrop(of: SidebarTabDrag.acceptedTypes, delegate: TabReorderDropDelegate(
                    target: .loose(index: store.looseTabs.count),
                    draggingID: $draggingTabID,
                    store: store,
                    isTargeted: $appendDropTargeted))
        }
    }
}

private struct SidebarDropCatchZone: View {
    let height: CGFloat
    let cornerRadius: CGFloat
    let isTargeted: Bool

    @Environment(\.palette) private var p

    var body: some View {
        RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
            .fill(isTargeted ? p.sidebarForeground.color.opacity(0.08) : .clear)
            .frame(maxWidth: .infinity)
            .frame(height: height)
            .contentShape(Rectangle())
    }
}

private struct NewTabRow: View {
    let action: () -> Void
    @Environment(\.palette) private var p
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            HStack(spacing: 9) {
                Icon(name: "plus", size: 15)
                    .foregroundStyle(p.mutedForeground.color)
                    .frame(width: 16)
                Text("New Tab")
                    .font(Typography.ui(Typography.base))
                    .foregroundStyle(p.mutedForeground.color)
                Spacer()
            }
            .padding(.horizontal, 9)
            .frame(height: 32)
            .background(
                RoundedRectangle(cornerRadius: TabSurface.radius, style: .continuous)
                    .fill(hovering ? p.foreground.color.opacity(0.05) : .clear)
            )
            .contentShape(Rectangle())
        }
        .buttonStyle(PressShrinkButtonStyle())
        .onHover { hovering = $0 }
    }
}

// MARK: - Tab context menu

/// Shared right-click menu for any tab row/tile.
struct TabMenu: View {
    @ObservedObject var store: BrowserStore
    let tab: BrowserTab

    var body: some View {
        Button(store.isPinned(tab.id) ? "Unpin" : "Pin") {
            store.togglePin(tab.id)
        }
        if !store.folders.isEmpty {
            Menu("Add to Folder") {
                ForEach(store.folders) { folder in
                    Button(folder.name) { store.addTab(tab.id, toFolder: folder.id) }
                }
            }
        }
        Button("New Folder with Tab") {
            let folder = store.addFolderForEditing()
            store.addTab(tab.id, toFolder: folder.id)
        }
        if store.folders.contains(where: { $0.tabIDs.contains(tab.id) }) {
            Button("Remove from Folder") { store.removeTabFromFolders(tab.id) }
        }
        Divider()
        Button("Duplicate Tab") { store.duplicateTab(tab.id) }
        Button("Copy URL") { store.copyURL(of: tab.id) }
        Divider()
        Button("Reload") { tab.reload() }
        Button("Close Other Tabs") { store.closeOtherTabs(than: tab.id) }
            .disabled(store.tabs.filter { $0.id != tab.id && !store.isPinned($0.id) }.isEmpty)
        Button("Close Tabs to Right") { store.closeTabsToRight(of: tab.id) }
            .disabled(!store.hasClosableTabsToRight(of: tab.id))
        Button("Close Tab", role: .destructive) { store.closeTab(tab.id) }
    }
}

// MARK: - Bottom bar

private struct SidebarBottomBar: View {
    @ObservedObject var store: BrowserStore

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 6) {
                IconButton(systemName: "mori",
                           kind: store.aiPanelVisible ? .primary : .ghost,
                           size: 30) { store.toggleAIPanel() }
                Spacer()
                ThemeSwatchButton()
                AppearanceToggle(store: store)
                IconButton(systemName: "gearshape", size: 30) { store.toggleSettings() }
                    .help("Settings")
            }
            .padding(.horizontal, 10)
            .frame(height: 46)
        }
    }
}

// MARK: - Shared small buttons

/// Light/dark toggle that flips the persisted theme preference.
struct AppearanceToggle: View {
    @ObservedObject var store: BrowserStore
    @ObservedObject private var settings = BrowserSettings.shared
    @Environment(\.colorScheme) private var systemScheme

    private var isDark: Bool {
        (settings.theme.colorScheme ?? systemScheme) == .dark
    }

    var body: some View {
        IconButton(systemName: isDark ? "sun.max" : "moon", size: 30) {
            settings.theme = isDark ? .light : .dark
        }
        .help("Toggle light / dark")
    }
}

/// A round color chip showing the current accent that opens the gradient theme
/// picker in a popover — the quick-access counterpart to the Settings panel.
struct ThemeSwatchButton: View {
    @ObservedObject private var settings = BrowserSettings.shared
    @Environment(\.palette) private var p
    @Environment(\.colorScheme) private var scheme
    @State private var showPicker = false

    /// The current theme as a soft chip. Uses the same blurred radial mesh as
    /// the gallery tiles and chrome wash so the colors blend smoothly instead of
    /// banding across a hard linear seam in such a small circle.
    @ViewBuilder private var swatch: some View {
        let theme = settings.gradientTheme
        let colors = theme.dots.map(\.rgb.color)
        if theme.isEmpty {
            Circle().fill(p.primary.color)
        } else if colors.count >= 2 {
            GradientMesh(colors: colors, relativeBlur: 0.5, maxBlur: 12)
                .clipShape(Circle())
        } else {
            Circle().fill(colors.first ?? p.primary.color)
        }
    }

    var body: some View {
        Button {
            showPicker.toggle()
        } label: {
            swatch
                .frame(width: 16, height: 16)
                .overlay(Circle().strokeBorder(p.border.color.opacity(0.6), lineWidth: 1))
                .frame(width: 30, height: 30)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help("Color theme")
        .popover(isPresented: $showPicker, arrowEdge: .bottom) {
            ThemePicker()
                .padding(16)
                .environment(\.palette, p)
                .preferredColorScheme(scheme)
        }
    }
}
