import SwiftUI
import AppKit

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
        self.store = store
        self.palette = palette
        self.scheme = scheme
        rebuild()

        let nowVisible = store.launcherVisible
        if nowVisible != visible {
            visible = nowVisible
            if nowVisible {
                // Pull keyboard focus away from the CEF page so the search field
                // receives typing the moment the palette appears.
                DispatchQueue.main.async { [weak self] in
                    guard let self, let host = self.hosting else { return }
                    self.window?.makeFirstResponder(host)
                }
            }
        }
    }

    private func rebuild() {
        guard let store else { return }
        hosting?.rootView = AnyView(
            Group {
                if store.launcherVisible {
                    LauncherView(store: store, scheme: scheme)
                        .environment(\.palette, palette)
                        .frame(width: max(bounds.width, 1),
                               height: max(bounds.height, 1),
                               alignment: .top)
                }
            }
        )
    }

    override func hitTest(_ point: NSPoint) -> NSView? {
        // Modal while open; otherwise let every click reach the web view.
        guard store?.launcherVisible == true else { return nil }
        return super.hitTest(point)
    }

    override func layout() {
        super.layout()
        hosting?.frame = bounds
        if visible { rebuild() }
    }
}

// MARK: - Palette UI

private struct LauncherView: View {
    @ObservedObject var store: BrowserStore
    var scheme: ColorScheme
    @Environment(\.palette) private var p

    @State private var query = ""
    @State private var highlighted = 0
    @FocusState private var fieldFocused: Bool

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
            // Seed from the address bar (current URL) when invoked there; blank
            // for a ⌘T launcher. Pre-select so the first keystroke replaces it.
            query = store.launcherPrefill
            highlighted = 0
            DispatchQueue.main.async {
                fieldFocused = true
                if !query.isEmpty {
                    DispatchQueue.main.async { selectAllField() }
                }
            }
        }
        .onChange(of: query) { _, _ in highlighted = 0 }
    }

    /// Select the launcher field's text so a pre-filled URL is replaced wholesale
    /// on the first keystroke (matching the old omnibox focus behavior).
    private func selectAllField() {
        if let editor = NSApp.keyWindow?.firstResponder as? NSTextView {
            editor.selectAll(nil)
        }
    }

    private var card: some View {
        VStack(spacing: 0) {
            header

            if !items.isEmpty {
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
                .shadow(color: .black.opacity(scheme == .dark ? 0.6 : 0.25), radius: 44, y: 22)
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
                        .font(Typography.ui(15))
                        .foregroundStyle(p.mutedForeground.color.opacity(0.65))
                }
                TextField("", text: $query)
                    .textFieldStyle(.plain)
                    .font(Typography.ui(15))
                    .foregroundStyle(p.foreground.color)
                    .tint(p.primary.color)
                    .focused($fieldFocused)
                    .onSubmit(commit)
            }
        }
        .padding(.horizontal, LauncherMetrics.headerPadding)
        .frame(height: LauncherMetrics.headerHeight)
    }

    private var results: some View {
        ScrollView {
            VStack(spacing: LauncherMetrics.rowSpacing) {
                ForEach(Array(items.enumerated()), id: \.element.id) { idx, item in
                    LauncherRow(item: item, isHighlighted: idx == highlighted, scheme: scheme) {
                        activate(item)
                    }
                    .onHover { if $0 { highlighted = idx } }
                }
            }
            .padding(.horizontal, LauncherMetrics.resultsPadding)
            .padding(.vertical, LauncherMetrics.resultsPadding)
        }
        .frame(maxHeight: LauncherMetrics.maxResultsHeight)
        .scrollIndicators(.never)
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

    private func activate(_ item: LauncherItem) {
        if let id = item.tabID {
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

    static func build(query: String, store: BrowserStore) -> [LauncherItem] {
        let q = query.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let rawQuery = query.trimmingCharacters(in: .whitespacesAndNewlines)
        var seen = Set<String>()
        var out: [LauncherItem] = []

        func favicon(for u: String) -> String? {
            guard let host = URL(string: u)?.host else { return nil }
            return "https://www.google.com/s2/favicons?domain=\(host)&sz=64"
        }

        if !rawQuery.isEmpty {
            let resolved = URLInterpreter.resolve(rawQuery, settings: store.settings)
            let isAddress = URLInterpreter.resolvesAsAddress(rawQuery)
            seen.insert(resolved)
            // Stable id (not keyed on the resolved URL) so the row — and its
            // Favicon/AsyncImage — persists across keystrokes instead of being
            // torn down and reloaded on every character. The search-engine
            // favicon is host-derived and therefore constant while typing, so a
            // persistent view keeps it rock-solid with no reload flash.
            out.append(LauncherItem(id: isAddress ? "direct-address" : "direct-search",
                                    title: isAddress ? "Open \(rawQuery)" : "Search \(rawQuery)",
                                    url: resolved,
                                    faviconURL: isAddress ? favicon(for: resolved) : nil,
                                    tabID: nil,
                                    action: isAddress ? "Open" : "Search"))
        }

        // Open tabs first — all of them when idle, filtered while typing. In
        // address-bar mode the current tab is the one being edited, so offering
        // to "Switch to" it would be redundant — skip it.
        for tab in store.tabs {
            if store.launcherEditsCurrentTab, tab.id == store.selectedTabID { continue }
            let match = q.isEmpty
                || tab.title.lowercased().contains(q)
                || tab.urlString.lowercased().contains(q)
            guard match else { continue }
            let key = tab.urlString.isEmpty ? "tab:\(tab.id)" : tab.urlString
            guard seen.insert(key).inserted else { continue }
            out.append(LauncherItem(id: "tab-\(tab.id)",
                                    title: tab.title,
                                    url: tab.displayURL,
                                    faviconURL: tab.faviconURL ?? favicon(for: tab.urlString),
                                    tabID: tab.id,
                                    action: "Switch to Tab"))
        }

        // Then history: recent when idle, best matches while typing.
        let history = q.isEmpty
            ? Array(HistoryStore.shared.entries.prefix(8))
            : HistoryStore.shared.suggestions(for: q, limit: 8)
        for entry in history {
            guard seen.insert(entry.url).inserted else { continue }
            out.append(LauncherItem(id: "hist-\(entry.id)",
                                    title: entry.title.isEmpty ? entry.url : entry.title,
                                    url: entry.url,
                                    faviconURL: favicon(for: entry.url),
                                    tabID: nil,
                                    action: "Open"))
        }

        return Array(out.prefix(7))
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
        item.tabID != nil || isHighlighted || hovering
    }

    var body: some View {
        Button(action: action) {
            HStack(spacing: 11) {
                Favicon(icon: item.faviconURL, page: item.url, size: 18)

                Text(item.title.isEmpty ? item.url : item.title)
                    .font(Typography.ui(13, weight: .medium))
                    .foregroundStyle(p.foreground.color)
                    .lineLimit(1)
                    .truncationMode(.tail)

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
    }

    private var trailing: some View {
        HStack(spacing: 7) {
            Text(item.action)
                .font(Typography.ui(11, weight: .medium))
                .foregroundStyle(isHighlighted ? p.foreground.color : p.mutedForeground.color.opacity(0.7))

            Icon(name: "arrow.right", size: 11, weight: .semibold)
                .foregroundStyle(isHighlighted ? p.popover.color : p.mutedForeground.color.opacity(0.7))
                .frame(width: 20, height: 20)
                .background(
                    RoundedRectangle(cornerRadius: 6, style: .continuous)
                        .fill(isHighlighted ? p.foreground.color : p.foreground.color.opacity(0.07))
                )
        }
        .fixedSize()
    }
}
