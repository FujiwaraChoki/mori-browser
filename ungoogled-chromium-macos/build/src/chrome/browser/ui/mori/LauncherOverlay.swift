import SwiftUI
import AppKit
import Combine

/// The new-tab launcher — a Spotlight-style command palette floated above the
/// web content. Triggered by ⌘T / the sidebar's "New Tab" row instead of
/// silently spawning a blank tab, it lets you search, jump to an already-open
/// tab, or pick from history before a tab is ever created.
///
/// Like the sidebar peek, this must be AppKit-hosted: the live CEF browser
/// composites *above* SwiftUI `.overlay`s and would otherwise cover the palette
/// and swallow its clicks. Hosting an `NSView` above the web view (and gating
/// `hitTest`) puts the palette on top and lets it take keyboard focus.
struct LauncherOverlay: NSViewRepresentable {
    @ObservedObject var store: BrowserStore
    var palette: ThemePalette
    var scheme: ColorScheme

    func makeNSView(context: Context) -> LauncherContainerView {
        let view = LauncherContainerView()
        view.update(store: store, palette: palette, scheme: scheme)
        return view
    }

    func updateNSView(_ nsView: LauncherContainerView, context: Context) {
        nsView.update(store: store, palette: palette, scheme: scheme)
    }
}

/// Hosts the palette UI above the web view and gates interaction via `hitTest`:
/// fully click-through when closed, modal (captures everything) when open.
final class LauncherContainerView: NSView {
    private var hosting: NSHostingView<AnyView>?
    private weak var store: BrowserStore?
    private var palette: ThemePalette = .light
    private var scheme: ColorScheme = .light
    private var visible = false
    /// Drives show/hide straight off `launcherVisible` instead of SwiftUI's
    /// `updateNSView` pass. A keyboard ⌘T mutates the store from outside SwiftUI,
    /// and the chrome flush forces synchronous layouts; that racing of forced
    /// layout against SwiftUI's representable reconcile made `updateNSView` read
    /// a *stale* `launcherVisible` on rapid toggles (open then close inside the
    /// ~0.35s flush window), so the palette got stuck open. The publisher always
    /// carries the authoritative new value synchronously on `willSet`, making
    /// the toggle reliable regardless of flush/layout timing.
    private var visibilityObserver: AnyCancellable?

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        wantsLayer = true
        let host = NSHostingView(rootView: AnyView(EmptyView()))
        host.frame = bounds
        host.autoresizingMask = [.width, .height]
        addSubview(host)
        hosting = host
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override var isFlipped: Bool { true }

    func update(store: BrowserStore, palette: ThemePalette, scheme: ColorScheme) {
        let storeChanged = self.store !== store
        self.store = store
        // Keep palette/scheme current so the *next* open is styled correctly;
        // visibility transitions are owned by the publisher subscription below,
        // not this (frequently re-invoked, and timing-racy) update pass.
        self.palette = palette
        self.scheme = scheme

        guard storeChanged else { return }
        // Subscribe once: a @Published publisher emits the current value on
        // subscribe, then the new value on every change — synchronously, so the
        // launcher can never be left out of sync with the store.
        visibilityObserver = store.$launcherVisible.sink { [weak self] newVisible in
            self?.applyVisible(newVisible)
        }
    }

    private func applyVisible(_ nowVisible: Bool) {
        guard nowVisible != visible else { return }
        visible = nowVisible
        rebuild(visible: nowVisible)
    }

    private func rebuild(visible: Bool) {
        guard let store else { return }
        hosting?.rootView = AnyView(
            Group {
                if visible {
                    LauncherView(store: store, scheme: scheme)
                        .environment(\.palette, palette)
                }
            }
        )
        // The toggle came from outside SwiftUI; draw the change now rather than
        // waiting for the next event to pump the run loop.
        needsLayout = true
        needsDisplay = true
    }

    override func hitTest(_ point: NSPoint) -> NSView? {
        // Modal while open; otherwise let every click reach the web view.
        guard visible else { return nil }
        return super.hitTest(point)
    }

    override func layout() {
        super.layout()
        hosting?.frame = bounds
    }
}

// MARK: - Palette UI

private struct LauncherView: View {
    @ObservedObject var store: BrowserStore
    var scheme: ColorScheme
    @Environment(\.palette) private var p

    @State private var query = ""
    @State private var highlighted = 0

    private var items: [LauncherItem] { LauncherItem.build(query: query, store: store) }

    var body: some View {
        GeometryReader { geo in
            ZStack(alignment: .top) {
                // Invisible click-outside target; the page behind the launcher
                // should stay visually unchanged.
                Color.clear
                    .contentShape(Rectangle())
                    .onTapGesture { store.dismissLauncher() }

                // Pin the card's *top* edge to a fixed fraction down from the
                // top of the window (Spotlight-style) so it only ever grows
                // downward — its position stays fixed regardless of how many
                // results are rendered.
                card
                    .frame(maxWidth: LauncherMetrics.cardWidth)
                    .padding(.horizontal, LauncherMetrics.horizontalPadding)
                    .padding(.top, geo.size.height * LauncherMetrics.topFraction)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .onAppear {
            resetForPresentation()
        }
        .onChange(of: store.launcherFocusRequest) { _, _ in
            resetForPresentation()
        }
        .onChange(of: query) { _, _ in highlighted = 0 }
    }

    private func resetForPresentation() {
        // Seed from the address bar (current URL) when invoked there; blank
        // for a Cmd-T launcher. Address-bar text is selected by the AppKit
        // field so the first keystroke replaces it wholesale.
        query = store.launcherPrefill
        highlighted = 0
    }

    private var card: some View {
        VStack(spacing: 0) {
            header

            if items.isEmpty {
                // Idle hint so the palette reads as capable, not broken, before
                // any keystroke surfaces results/commands.
                Text("Type to search, open a URL, or run a command")
                    .font(Typography.ui(Typography.small))
                    .foregroundStyle(p.mutedForeground.color.opacity(0.55))
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, LauncherMetrics.headerPadding)
                    .padding(.vertical, Spacing.xl)
            } else {
                Rectangle()
                    .fill(p.border.color.opacity(0.4))
                    .frame(height: 1)
                    .padding(.horizontal, LauncherMetrics.headerPadding)
                results
            }
        }
        .background(
            RoundedRectangle(cornerRadius: LauncherMetrics.cornerRadius, style: .continuous)
                .fill(p.popover.color)
                .elevation(.modal, scheme)
        )
        .overlay(
            RoundedRectangle(cornerRadius: LauncherMetrics.cornerRadius, style: .continuous)
                .strokeBorder(
                    LinearGradient(
                        colors: scheme == .dark
                            ? [.white.opacity(0.1), .white.opacity(0.03)]
                            : [.black.opacity(0.06), .black.opacity(0.02)],
                        startPoint: .top, endPoint: .bottom
                    ),
                    lineWidth: 1
                )
        )
        // Swallow taps on the card so they don't fall through to the scrim.
        .contentShape(RoundedRectangle(cornerRadius: LauncherMetrics.cornerRadius, style: .continuous))
        .onTapGesture {}
        .onKeyPress(.downArrow) { move(1); return .handled }
        .onKeyPress(.upArrow) { move(-1); return .handled }
        .onKeyPress(.escape) { store.dismissLauncher(); return .handled }
    }

    private var header: some View {
        HStack(spacing: 11) {
            Icon(name: "magnifyingglass", size: 16, weight: .medium)
                .foregroundStyle(p.mutedForeground.color.opacity(0.65))

            ZStack(alignment: .leading) {
                if query.isEmpty {
                    Text("Search or Enter URL…")
                        .font(Typography.ui(Typography.title))
                        .foregroundStyle(p.mutedForeground.color.opacity(0.65))
                }
                LauncherSearchField(text: $query,
                                    focusRequest: store.launcherFocusRequest,
                                    selectAllOnFocus: !store.launcherPrefill.isEmpty,
                                    foregroundColor: p.foreground.nsColor,
                                    insertionColor: p.primary.nsColor,
                                    onMove: move,
                                    onEscape: store.dismissLauncher,
                                    onSubmit: commit,
                                    onShortcut: handleShortcut,
                                    onTab: agentLaunchHandler)
                    .frame(height: 24)
            }
            .frame(maxWidth: .infinity, alignment: .leading)

            // Quiet hint that ⇥ launches an agent task from the typed text.
            if store.settings.aiIntegrationEnabled {
                agentHint
            }
        }
        .padding(.horizontal, LauncherMetrics.headerPadding)
        .frame(height: LauncherMetrics.headerHeight)
    }

    /// Trailing affordance in the search row: a key-cap "TAB" plus a muted
    /// "for Agent" label, advertising the ⇥-to-launch-an-agent gesture. Purely
    /// informational, so it never steals the pointer from the field.
    private var agentHint: some View {
        HStack(spacing: 6) {
            Text("TAB")
                .font(Typography.ui(Typography.caption, weight: .semibold))
                .foregroundStyle(p.mutedForeground.color)
                .padding(.horizontal, 5)
                .padding(.vertical, 2)
                .background(
                    RoundedRectangle(cornerRadius: Radius.sm, style: .continuous)
                        .fill(p.foreground.color.opacity(0.07))
                )
            Text("for Agent")
                .font(Typography.ui(Typography.small, weight: .medium))
                .foregroundStyle(p.mutedForeground.color.opacity(0.7))
        }
        .fixedSize()
        .allowsHitTesting(false)
    }

    private var agentLaunchHandler: ((String) -> Void)? {
        guard store.settings.aiIntegrationEnabled else { return nil }
        return { store.launcherLaunchAgent($0) }
    }

    private var results: some View {
        ScrollViewReader { proxy in
            ScrollView {
                VStack(spacing: LauncherMetrics.rowSpacing) {
                    ForEach(Array(items.enumerated()), id: \.element.id) { idx, item in
                        LauncherRow(item: item, isHighlighted: idx == highlighted, scheme: scheme) {
                            activate(item)
                        }
                        .id(idx)
                        .onHover { if $0 { highlighted = idx } }
                    }
                }
                .padding(.horizontal, LauncherMetrics.resultsPadding)
                .padding(.vertical, LauncherMetrics.resultsPadding)
            }
            .frame(maxHeight: LauncherMetrics.maxResultsHeight)
            .scrollIndicators(.never)
            // Keep the keyboard-highlighted row visible; minimal scroll (no
            // anchor) so navigating among already-visible rows never yanks the
            // list — and never fights the rows' own hover-to-highlight.
            .onChange(of: highlighted) { _, new in proxy.scrollTo(new) }
        }
    }

    private func move(_ delta: Int) {
        guard !items.isEmpty else { return }
        highlighted = (highlighted + delta + items.count) % items.count
    }

    private func commit() {
        if items.indices.contains(highlighted) {
            activate(items[highlighted])
        } else {
            store.launcherOpen(query)
        }
    }

    private func handleShortcut(_ trigger: MoriShortcutTrigger) -> Bool {
        guard let item = items.first(where: { $0.shortcutHint?.matches(trigger) == true }) else {
            return false
        }
        activate(item)
        return true
    }

    private func activate(_ item: LauncherItem) {
        if let run = item.run {
            run()
        } else if let id = item.tabID {
            store.launcherSwitch(to: id)
        } else {
            store.launcherOpen(url: item.url)
        }
    }
}

private enum LauncherMetrics {
    static let cardWidth: CGFloat = 620
    static let horizontalPadding: CGFloat = 24
    static let headerHeight: CGFloat = 52
    static let headerPadding: CGFloat = 16
    static let rowHeight: CGFloat = 48
    static let rowSpacing: CGFloat = 1
    static let resultsPadding: CGFloat = 6
    static let rowInnerPadding: CGFloat = 10
    static let rowCorner: CGFloat = 8
    static let visibleResultCount = 6
    static let maxResultsHeight: CGFloat = {
        let rows = CGFloat(visibleResultCount)
        let gaps = CGFloat(max(visibleResultCount - 1, 0))
        return rows * rowHeight + gaps * rowSpacing + resultsPadding * 2
    }()
    static let cornerRadius: CGFloat = Radius.popover
    /// Fraction of the window height at which the card's top edge is pinned.
    static let topFraction: CGFloat = 0.24

    /// The highlighted-row wash — a touch of light over the card surface so the
    /// active result reads clearly without a heavy accent tint.
    static func highlightFill(_ scheme: ColorScheme) -> Color {
        scheme == .dark ? .white.opacity(0.07) : .black.opacity(0.05)
    }
}

/// AppKit-backed launcher input. SwiftUI `@FocusState` is timing-sensitive when
/// hosted above Chromium's native view; the field editor here can claim first
/// responder directly on each presentation and keep normal palette keys working.
private struct LauncherSearchField: NSViewRepresentable {
    @Binding var text: String
    let focusRequest: Int
    let selectAllOnFocus: Bool
    let foregroundColor: NSColor
    let insertionColor: NSColor
    let onMove: (Int) -> Void
    let onEscape: () -> Void
    let onSubmit: () -> Void
    let onShortcut: (MoriShortcutTrigger) -> Bool
    /// Pressing Tab (instead of Return) hands the live field text off to kick
    /// off an agent task. Receives the field's current string directly so a
    /// keystroke-then-Tab in the same runloop isn't missing its last character.
    var onTab: ((String) -> Void)? = nil

    func makeCoordinator() -> Coordinator {
        Coordinator(self)
    }

    func makeNSView(context: Context) -> LauncherTextField {
        let field = LauncherTextField(frame: .zero)
        field.isBordered = false
        field.drawsBackground = false
        field.backgroundColor = .clear
        field.focusRingType = .none
        field.usesSingleLineMode = true
        field.isEditable = true
        field.isSelectable = true
        field.font = Self.font
        field.textColor = foregroundColor
        field.delegate = context.coordinator
        field.onKeyDown = { [weak coordinator = context.coordinator] event in
            coordinator?.handleKeyDown(event) ?? false
        }
        field.cell?.wraps = false
        field.cell?.isScrollable = true
        field.cell?.lineBreakMode = .byClipping
        return field
    }

    func updateNSView(_ field: LauncherTextField, context: Context) {
        context.coordinator.parent = self
        if field.stringValue != text {
            field.stringValue = text
        }
        field.font = Self.font
        field.textColor = foregroundColor
        field.backgroundColor = .clear
        context.coordinator.focusIfNeeded(field)
    }

    final class LauncherTextField: NSTextField {
        var onKeyDown: ((NSEvent) -> Bool)?

        override func keyDown(with event: NSEvent) {
            if onKeyDown?(event) == true { return }
            super.keyDown(with: event)
        }
    }

    private static var font: NSFont {
        if let family = FontRegistry.soehneFamily,
           let font = NSFont(name: family, size: 15) {
            return font
        }
        return .systemFont(ofSize: 15)
    }

    final class Coordinator: NSObject, NSTextFieldDelegate {
        var parent: LauncherSearchField
        private var appliedFocusRequest: Int?

        init(_ parent: LauncherSearchField) {
            self.parent = parent
        }

        func controlTextDidChange(_ notification: Notification) {
            guard let field = notification.object as? NSTextField else { return }
            parent.text = field.stringValue
        }

        func handleKeyDown(_ event: NSEvent) -> Bool {
            let trigger = MoriShortcutTrigger(event: event)
            guard !trigger.modifiers.isEmpty else { return false }
            return parent.onShortcut(trigger)
        }

        func control(_ control: NSControl,
                     textView: NSTextView,
                     doCommandBy commandSelector: Selector) -> Bool {
            switch commandSelector {
            case #selector(NSResponder.insertNewline(_:)):
                parent.onSubmit()
                return true
            case #selector(NSResponder.moveDown(_:)):
                parent.onMove(1)
                return true
            case #selector(NSResponder.moveUp(_:)):
                parent.onMove(-1)
                return true
            case #selector(NSResponder.cancelOperation(_:)):
                parent.onEscape()
                return true
            case #selector(NSResponder.insertTab(_:)):
                // Tab launches an agent task from the typed text. Consume it so
                // focus never shifts away; an empty field is a no-op.
                guard let onTab = parent.onTab else { return false }
                let typed = textView.string.trimmingCharacters(in: .whitespacesAndNewlines)
                if !typed.isEmpty { onTab(textView.string) }
                return true
            default:
                return false
            }
        }

        func focusIfNeeded(_ field: NSTextField) {
            let request = parent.focusRequest
            guard appliedFocusRequest != request else {
                applyInsertionColor(field)
                return
            }

            applyFocus(field, request: request)
            DispatchQueue.main.async { [weak self, weak field] in
                guard let self, let field else { return }
                self.applyFocus(field, request: request)
                DispatchQueue.main.async { [weak self, weak field] in
                    guard let self, let field else { return }
                    self.applyFocus(field, request: request)
                }
            }
        }

        private func applyFocus(_ field: NSTextField, request: Int) {
            guard let window = field.window else { return }
            window.makeFirstResponder(field)
            applyInsertionColor(field)

            guard field.currentEditor() != nil || window.firstResponder === field else {
                return
            }
            if parent.selectAllOnFocus {
                field.currentEditor()?.selectAll(nil)
            }
            appliedFocusRequest = request
        }

        private func applyInsertionColor(_ field: NSTextField) {
            (field.currentEditor() as? NSTextView)?.insertionPointColor = parent.insertionColor
        }
    }
}

/// One launcher result: either an open tab (offers "Switch to Tab") or a history
/// entry (opens in a fresh tab).
private struct LauncherItem: Identifiable {
    let id: String
    let title: String
    let url: String
    let faviconURL: String?
    /// Non-nil when this result is an already-open tab.
    let tabID: BrowserTab.ID?
    /// Trailing affordance label ("Switch to Tab", "Open", "Search").
    let action: String
    /// For symbol-backed results: the SF Symbol to show in place of a favicon.
    var iconSystemName: String? = nil
    /// For command results: the action to run on activation. Command closures
    /// dismiss the launcher themselves.
    var run: (() -> Void)? = nil
    /// Visible command-key equivalent. Pressing it while the launcher field is
    /// focused runs this row directly, matching the badge.
    var shortcutHint: MoriShortcutHint? = nil

    private struct RankedItem {
        let item: LauncherItem
        let score: Int
        let sourceBonus: Int
        let recency: Date

        var rank: Int { score + sourceBonus }
    }

    static func build(query: String, store: BrowserStore) -> [LauncherItem] {
        let rawQuery = query.trimmingCharacters(in: .whitespacesAndNewlines)
        let q = rawQuery.lowercased()
        var seen = Set<String>()
        var leading: [LauncherItem] = []
        var ranked: [RankedItem] = []

        if !rawQuery.isEmpty {
            let resolved = URLInterpreter.resolve(rawQuery, settings: store.settings)
            let isAddress = URLInterpreter.resolvesAsAddress(rawQuery)
            seen.insert(resolved)
            // Stable id (not keyed on the resolved URL) so the row persists
            // across keystrokes instead of being torn down on every character.
            leading.append(LauncherItem(id: isAddress ? "direct-address" : "direct-search",
                                        title: isAddress ? "Open \(rawQuery)" : "Search \(rawQuery)",
                                        url: resolved,
                                        faviconURL: nil,
                                        tabID: nil,
                                        action: isAddress ? "Open" : "Search"))
        }

        // Open tabs first when idle, then scored with the rest while typing. In
        // address-bar mode the current tab is the one being edited, so offering
        // to "Switch to" it would be redundant — skip it.
        for tab in store.tabs {
            if store.launcherEditsCurrentTab, tab.id == store.selectedTabID { continue }
            let score = q.isEmpty
                ? 0
                : matchScore(query: q, title: tab.displayTitle, detail: tab.urlString)
            guard let score else { continue }
            let key = tab.urlString.isEmpty ? "tab:\(tab.id)" : tab.urlString
            guard seen.insert(key).inserted else { continue }
            ranked.append(RankedItem(item: LauncherItem(id: "tab-\(tab.id)",
                                                        title: tab.displayTitle,
                                                        url: tab.displayURL,
                                                        faviconURL: tab.faviconURL,
                                                        tabID: tab.id,
                                                        action: "Switch to Tab"),
                                     score: score,
                                     sourceBonus: 70,
                                     recency: tab.lastAccessedAt))
        }

        // Then history: recent when idle, scored against the full store while typing.
        let history = q.isEmpty ? Array(HistoryStore.shared.entries.prefix(8)) : HistoryStore.shared.entries
        for entry in history {
            let score = q.isEmpty
                ? 0
                : matchScore(query: q, title: entry.title, detail: entry.url)
            guard let score else { continue }
            guard seen.insert(entry.url).inserted else { continue }
            ranked.append(RankedItem(item: LauncherItem(id: "hist-\(entry.id)",
                                                        title: entry.title.isEmpty ? entry.url : entry.title,
                                                        url: entry.url,
                                                        faviconURL: nil,
                                                        tabID: nil,
                                                        action: "Open"),
                                     score: score,
                                     sourceBonus: 55,
                                     recency: entry.lastVisited))
        }

        if !q.isEmpty {
            for mark in BookmarkStore.shared.bookmarks {
                guard let score = matchScore(query: q, title: mark.title, detail: mark.url),
                      seen.insert(mark.url).inserted else { continue }
                ranked.append(RankedItem(item: LauncherItem(id: "bookmark-\(mark.id)",
                                                            title: mark.title.isEmpty ? mark.url : mark.title,
                                                            url: mark.url,
                                                            faviconURL: nil,
                                                            tabID: nil,
                                                            action: "Open",
                                                            iconSystemName: "star"),
                                         score: score,
                                         sourceBonus: 45,
                                         recency: mark.createdAt))
            }

            ranked.append(contentsOf: commands(query: q, store: store))
        }

        ranked.sort {
            if $0.rank != $1.rank { return $0.rank > $1.rank }
            if $0.recency != $1.recency { return $0.recency > $1.recency }
            return $0.item.title.localizedCaseInsensitiveCompare($1.item.title) == .orderedAscending
        }

        let askAI: LauncherItem? = !rawQuery.isEmpty
            ? LauncherItem(id: "ask-ai",
                           title: "Ask AI: \(rawQuery)",
                           url: rawQuery,
                           faviconURL: nil,
                           tabID: nil,
                           action: "Ask",
                           iconSystemName: "sparkles",
                           run: {
                               Task { @MainActor in
                                   store.launcherLaunchAgent(rawQuery)
                               }
                           })
            : nil

        var out = leading
        let available = max(0, 8 - out.count - (askAI == nil ? 0 : 1))
        out.append(contentsOf: ranked.prefix(available).map(\.item))
        if let askAI { out.append(askAI) }
        return Array(out.prefix(8))
    }

    /// Build the matching command (action) results for the current query.
    private static func commands(query q: String, store: BrowserStore) -> [RankedItem] {
        guard !q.isEmpty else { return [] }
        struct Cmd { let title: String; let icon: String; let keywords: String; let run: () -> Void }
        var defs: [Cmd] = [
            Cmd(title: "New Tab", icon: "plus.square", keywords: "new tab open") {
                store.dismissLauncher(); store.newTab() },
            Cmd(title: "New Split", icon: "rectangle.split.2x1", keywords: "split view side") {
                store.dismissLauncher(); store.newSplit() },
            Cmd(title: "Reader View", icon: "doc.plaintext", keywords: "reader read article") {
                store.dismissLauncher(); store.toggleReader() },
            Cmd(title: "Capture Region", icon: "camera.viewfinder", keywords: "screenshot capture region snip crop") {
                store.dismissLauncher(); store.startRegionCapture() },
            Cmd(title: "Capture Visible Tab", icon: "camera", keywords: "screenshot capture visible page") {
                store.dismissLauncher(); store.captureVisibleArea() },
            Cmd(title: "Boost This Site", icon: "wand.and.stars", keywords: "boost custom css js") {
                store.dismissLauncher(); store.presentBoostEditor() },
            Cmd(title: "Zap an Element", icon: "scope", keywords: "zap hide remove element") {
                store.dismissLauncher(); store.startZapMode() },
            Cmd(title: "Peek a Link", icon: "eye", keywords: "peek preview clipboard little arc") {
                store.dismissLauncher(); store.peekFromClipboardOrCurrent() },
            Cmd(title: "Sleep Background Tabs", icon: "moon.zzz", keywords: "sleep memory tabs free") {
                store.dismissLauncher(); store.sleepBackgroundTabs() },
            Cmd(title: "Reopen Closed Tab", icon: "arrow.uturn.left", keywords: "reopen closed restore tab") {
                store.dismissLauncher(); store.reopenClosedTab() },
            Cmd(title: "Find in Page", icon: "magnifyingglass", keywords: "find search page text") {
                store.dismissLauncher(); store.showFindBar() },
            Cmd(title: "Toggle Sidebar", icon: "sidebar.right", keywords: "sidebar hide show") {
                store.dismissLauncher(); store.toggleSidebar() },
            Cmd(title: "Settings", icon: "gearshape", keywords: "settings preferences options") {
                store.dismissLauncher(); store.settingsVisible = true },
            Cmd(title: "New Space", icon: "square.grid.2x2", keywords: "space context new create") {
                store.dismissLauncher(); store.contextCreationVisible = true },
            Cmd(title: "Rename Space", icon: "pencil", keywords: "rename space context title") {
                store.dismissLauncher(); store.contextRenamePending = true },
            Cmd(title: "Clear Finished Downloads", icon: "trash",
                keywords: "clear downloads finished remove") {
                store.dismissLauncher(); DownloadStore.shared.clearFinished() }
        ]

        // Batch action when a ⌘/⇧-click multi-selection exists.
        if store.multiSelectedTabIDs.count >= 2 {
            defs.append(Cmd(title: "New Folder with \(store.multiSelectedTabIDs.count) Selected Tabs",
                            icon: "folder.badge.plus",
                            keywords: "folder group selected tabs multi new") {
                store.dismissLauncher(); store.newFolderWithSelectedTabs() })
        }

        // Current-tab actions — only meaningful when a tab is selected.
        if let id = store.selectedTabID, let tab = store.selectedTab {
            defs.append(Cmd(title: "Copy URL", icon: "link", keywords: "copy url link address") {
                store.dismissLauncher(); store.copyCurrentTabURL() })
            defs.append(Cmd(title: store.isPinned(id) ? "Unpin Tab" : "Pin Tab",
                            icon: store.isPinned(id) ? "pin.slash" : "pin",
                            keywords: "pin unpin tab favorite") {
                store.dismissLauncher(); store.togglePin(id) })
            defs.append(Cmd(title: "Rename Tab", icon: "pencil", keywords: "rename tab title name") {
                store.dismissLauncher(); store.beginTabRename(id) })
            defs.append(Cmd(title: "Duplicate Tab", icon: "plus.square.on.square",
                            keywords: "duplicate tab copy clone") {
                store.dismissLauncher(); store.duplicateTab(id) })
            defs.append(Cmd(title: BookmarkStore.shared.isBookmarked(tab.urlString)
                                ? "Remove Bookmark" : "Bookmark Page",
                            icon: "star", keywords: "bookmark save star favorite") {
                store.dismissLauncher(); store.toggleBookmark() })
            // Move the current tab to another space.
            for ctx in store.contexts where ctx.id != store.activeContextID {
                defs.append(Cmd(title: "Move Tab to \(ctx.name)",
                                icon: "arrow.right.square",
                                keywords: "move tab space context \(ctx.name)") {
                    store.dismissLauncher(); store.moveTab(id, toContext: ctx.id, activate: true) })
            }
        }
        if store.settings.aiIntegrationEnabled {
            defs.append(Cmd(title: "Open Assistant", icon: "sparkles", keywords: "ai assistant codex chat ask") {
                store.dismissLauncher(); store.openAIPanel()
            })
        }
        for ctx in store.contexts where ctx.id != store.activeContextID {
            defs.append(Cmd(title: "Switch to \(ctx.name)",
                            icon: "arrow.right.circle",
                            keywords: "space context switch go \(ctx.name)") {
                store.dismissLauncher(); store.switchContext(to: ctx.id)
            })
        }

        return defs.compactMap { cmd in
            guard var score = matchScore(query: q, title: cmd.title, detail: cmd.keywords) else {
                return nil
            }
            if q.count < 3 {
                guard q.count > 1,
                      let titleScore = matchScore(query: q, title: cmd.title),
                      titleScore >= 400 else { return nil }
                score = min(score, 240)
            }
            return RankedItem(item: LauncherItem(id: "cmd-\(cmd.title)",
                                                 title: cmd.title,
                                                 url: "",
                                                 faviconURL: nil,
                                                 tabID: nil,
                                                 action: "Run",
                                                 iconSystemName: cmd.icon,
                                                 run: cmd.run,
                                                 shortcutHint: commandShortcutHint(for: cmd.title,
                                                                                   store: store)),
                              score: score,
                              sourceBonus: 0,
                              recency: .distantPast)
        }
    }

    private static func commandShortcutHint(for title: String,
                                            store: BrowserStore) -> MoriShortcutHint? {
        let shortcutID: String?
        switch title {
        case "New Split":
            shortcutID = "newSplit"
        case "Boost This Site":
            shortcutID = "boostSite"
        case "Peek a Link":
            shortcutID = "peek"
        case "Sleep Background Tabs":
            shortcutID = "sleepBackgroundTabs"
        case "Reopen Closed Tab":
            shortcutID = "reopenClosedTab"
        case "Find in Page":
            shortcutID = store.findBarVisible ? nil : "find"
        case "Toggle Sidebar":
            shortcutID = "toggleSidebar"
        case "Settings":
            shortcutID = "settings"
        case "Copy URL":
            shortcutID = "copyCurrentURL"
        case "Pin Tab", "Unpin Tab":
            shortcutID = "togglePinTab"
        case "Duplicate Tab":
            shortcutID = "duplicateTab"
        case "Bookmark Page", "Remove Bookmark":
            shortcutID = "bookmarkPage"
        case "Open Assistant":
            shortcutID = store.aiPanelVisible ? nil : "toggleAI"
        default:
            shortcutID = nil
        }

        guard let shortcutID else { return nil }
        return MoriCommands.shortcutHint(for: shortcutID)
    }

    private static func matchScore(query: String, title: String, detail: String = "") -> Int? {
        let tokens = query.lowercased().split(separator: " ").map(String.init)
        guard !tokens.isEmpty else { return nil }

        let title = title.lowercased()
        let detail = detail.lowercased()

        func score(_ needle: String, in haystack: String) -> Int? {
            guard !needle.isEmpty, !haystack.isEmpty else { return nil }
            if haystack.hasPrefix(needle) { return 400 }
            if hasWordBoundaryMatch(needle, in: haystack) { return 340 }
            if haystack.contains(needle) { return 260 }
            if needle.count >= 3, isSubsequence(needle, of: haystack) { return 180 }
            return nil
        }

        func hasWordBoundaryMatch(_ needle: String, in haystack: String) -> Bool {
            var start = haystack.startIndex
            while start < haystack.endIndex,
                  let range = haystack.range(of: needle, range: start..<haystack.endIndex) {
                if range.lowerBound == haystack.startIndex { return true }
                let before = haystack[haystack.index(before: range.lowerBound)]
                let isBoundary = before.unicodeScalars.allSatisfy {
                    !CharacterSet.alphanumerics.contains($0)
                }
                if isBoundary { return true }
                start = range.upperBound
            }
            return false
        }

        func isSubsequence(_ needle: String, of haystack: String) -> Bool {
            var index = haystack.startIndex
            for character in needle {
                guard let found = haystack[index...].firstIndex(of: character) else {
                    return false
                }
                index = haystack.index(after: found)
            }
            return true
        }

        var total = 0
        for token in tokens {
            let titleScore = score(token, in: title)
            let detailScore = score(token, in: detail).map { max($0 - 25, 0) }
            guard let best = [titleScore, detailScore].compactMap({ $0 }).max() else {
                return nil
            }
            total += best
        }
        return total / tokens.count
    }
}

private struct LauncherRow: View {
    let item: LauncherItem
    let isHighlighted: Bool
    let scheme: ColorScheme
    let action: () -> Void

    @Environment(\.palette) private var p
    @State private var hovering = false

    /// Tab rows advertise "Switch to Tab" at all times (dimmed at rest);
    /// open/search rows only reveal their affordance once active.
    private var showsAction: Bool {
        item.tabID != nil || item.shortcutHint != nil || isHighlighted || hovering
    }

    var body: some View {
        Button(action: action) {
            HStack(spacing: 11) {
                if let sys = item.iconSystemName {
                    Icon(name: sys, size: 16)
                        .foregroundStyle(p.foreground.color.opacity(0.85))
                        .frame(width: 18, height: 18)
                } else {
                    Favicon(icon: item.faviconURL, page: item.url, size: 18)
                }

                Text(item.title.isEmpty ? item.url : item.title)
                    .font(Typography.ui(Typography.base, weight: .medium))
                    .foregroundStyle(p.foreground.color)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .help(item.title.isEmpty ? item.url : item.title)

                Spacer(minLength: 12)

                if showsAction { trailing }
            }
            .padding(.horizontal, LauncherMetrics.rowInnerPadding)
            .frame(height: LauncherMetrics.rowHeight)
            .background(
                RoundedRectangle(cornerRadius: LauncherMetrics.rowCorner, style: .continuous)
                    .fill(isHighlighted ? LauncherMetrics.highlightFill(scheme) : .clear)
            )
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .animation(Motion.state, value: isHighlighted)
        .animation(Motion.state, value: hovering)
    }

    private var trailing: some View {
        HStack(spacing: 7) {
            Text(item.action)
                .font(Typography.ui(Typography.small, weight: .medium))
                .foregroundStyle(isHighlighted ? p.foreground.color : p.mutedForeground.color.opacity(0.7))

            LauncherShortcutKeycaps(hint: item.shortcutHint ?? .defaultAction,
                                    isHighlighted: isHighlighted)
        }
        .fixedSize()
    }
}

private struct LauncherShortcutKeycaps: View {
    let hint: MoriShortcutHint
    let isHighlighted: Bool

    @Environment(\.palette) private var p

    var body: some View {
        HStack(spacing: 4) {
            ForEach(hint.labels, id: \.self) { label in
                Text(label)
                    .font(Typography.ui(Typography.caption, weight: .semibold))
                    .foregroundStyle(foreground)
                    .lineLimit(1)
                    .minimumScaleFactor(0.75)
                    .frame(width: keyWidth(for: label), height: 20)
                    .padding(.horizontal, horizontalPadding(for: label))
                    .background(
                        RoundedRectangle(cornerRadius: Radius.md, style: .continuous)
                            .fill(background)
                    )
                    .overlay(
                        RoundedRectangle(cornerRadius: Radius.md, style: .continuous)
                            .strokeBorder(border, lineWidth: 1)
                    )
            }
        }
    }

    private var foreground: Color {
        isHighlighted ? p.popover.color : p.mutedForeground.color.opacity(0.75)
    }

    private var background: Color {
        isHighlighted ? p.foreground.color : p.foreground.color.opacity(0.07)
    }

    private var border: Color {
        isHighlighted ? p.foreground.color.opacity(0.35) : p.foreground.color.opacity(0.1)
    }

    private func keyWidth(for label: String) -> CGFloat {
        label.count > 1 ? 22 : 20
    }

    private func horizontalPadding(for label: String) -> CGFloat {
        label.count > 1 ? 4 : 0
    }
}
