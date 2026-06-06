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
            highlighted = 0
            DispatchQueue.main.async { fieldFocused = true }
        }
        .onChange(of: query) { _, _ in highlighted = 0 }
    }

    private var card: some View {
        VStack(spacing: 0) {
            header

            if !items.isEmpty {
                Divider()
                    .overlay(p.border.color.opacity(0.45))
                    .padding(.horizontal, 14)
                results
            }
        }
        .background(
            RoundedRectangle(cornerRadius: LauncherMetrics.cornerRadius, style: .continuous)
                .fill(p.popover.color)
                .shadow(color: .black.opacity(scheme == .dark ? 0.48 : 0.22), radius: 30, y: 14)
        )
        .overlay(
            RoundedRectangle(cornerRadius: LauncherMetrics.cornerRadius, style: .continuous)
                .strokeBorder(p.border.color.opacity(0.5), lineWidth: 1)
        )
        // Swallow taps on the card so they don't fall through to the scrim.
        .contentShape(RoundedRectangle(cornerRadius: LauncherMetrics.cornerRadius, style: .continuous))
        .onTapGesture {}
        .onKeyPress(.downArrow) { move(1); return .handled }
        .onKeyPress(.upArrow) { move(-1); return .handled }
        .onKeyPress(.escape) { store.dismissLauncher(); return .handled }
    }

    private var header: some View {
        HStack(spacing: 10) {
            Icon(name: "magnifyingglass", size: 15, weight: .regular)
                .foregroundStyle(p.mutedForeground.color)

            ZStack(alignment: .leading) {
                if query.isEmpty {
                    Text("Search…")
                        .font(Typography.ui(15))
                        .foregroundStyle(p.mutedForeground.color.opacity(0.7))
                }
                TextField("", text: $query)
                    .textFieldStyle(.plain)
                    .font(Typography.ui(15))
                    .foregroundStyle(p.foreground.color)
                    .focused($fieldFocused)
                    .onSubmit(commit)
            }

            Text("\(store.selectedTab?.zoomPercent ?? 100)%")
                .font(Typography.ui(11, weight: .medium))
                .foregroundStyle(p.mutedForeground.color)
                .monospacedDigit()
                .padding(.horizontal, 7)
                .frame(height: 22)
                .background(
                    Capsule()
                        .fill(p.foreground.color.opacity(0.06))
                )
        }
        .padding(.horizontal, 16)
        .frame(height: LauncherMetrics.headerHeight)
    }

    private var results: some View {
        ScrollView {
            VStack(spacing: LauncherMetrics.rowSpacing) {
                ForEach(Array(items.enumerated()), id: \.element.id) { idx, item in
                    LauncherRow(item: item, isHighlighted: idx == highlighted) {
                        activate(item)
                    }
                    .onHover { if $0 { highlighted = idx } }
                }
            }
            .padding(LauncherMetrics.resultsPadding)
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
    static let headerHeight: CGFloat = 48
    static let rowHeight: CGFloat = 46
    static let rowSpacing: CGFloat = 1
    static let resultsPadding: CGFloat = 6
    static let visibleResultCount = 5
    static let maxResultsHeight: CGFloat = {
        let rows = CGFloat(visibleResultCount)
        let gaps = CGFloat(max(visibleResultCount - 1, 0))
        return rows * rowHeight + gaps * rowSpacing + resultsPadding * 2
    }()
    static let cornerRadius: CGFloat = Radius.popover
    /// Fraction of the window height at which the card's top edge is pinned.
    static let topFraction: CGFloat = 0.28
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
            out.append(LauncherItem(id: "direct-\(resolved)",
                                    title: isAddress ? "Open \(rawQuery)" : "Search \(rawQuery)",
                                    url: resolved,
                                    faviconURL: isAddress ? favicon(for: resolved) : nil,
                                    tabID: nil))
        }

        // Open tabs first — all of them when idle, filtered while typing.
        for tab in store.tabs {
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
                                    tabID: tab.id))
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
                                    tabID: nil))
        }

        return Array(out.prefix(7))
    }
}

private struct LauncherRow: View {
    let item: LauncherItem
    let isHighlighted: Bool
    let action: () -> Void

    @Environment(\.palette) private var p
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            HStack(spacing: 10) {
                Favicon(icon: item.faviconURL, page: item.url, size: 18)

                VStack(alignment: .leading, spacing: 2) {
                    Text(item.title.isEmpty ? item.url : item.title)
                        .font(Typography.ui(13, weight: .medium))
                        .foregroundStyle(p.foreground.color)
                        .lineLimit(1)
                        .truncationMode(.tail)

                    if !item.url.isEmpty {
                        Text(item.url)
                            .font(Typography.ui(11))
                            .foregroundStyle(p.mutedForeground.color)
                            .lineLimit(1)
                            .truncationMode(.tail)
                    }
                }

                Spacer(minLength: 12)

                if item.tabID != nil, isHighlighted || hovering {
                    HStack(spacing: 6) {
                        Text("Switch to Tab")
                            .font(Typography.ui(11, weight: .medium))
                            .foregroundStyle(p.mutedForeground.color)
                        Icon(name: "arrow.right", size: 11, weight: .semibold)
                            .foregroundStyle(p.foreground.color.opacity(0.7))
                            .frame(width: 20, height: 20)
                            .background(
                                RoundedRectangle(cornerRadius: 6, style: .continuous)
                                    .fill(p.foreground.color.opacity(0.08))
                            )
                    }
                    .fixedSize()
                }
            }
            .padding(.horizontal, 10)
            .frame(height: LauncherMetrics.rowHeight)
            .background(
                RoundedRectangle(cornerRadius: 8, style: .continuous)
                    .fill(isHighlighted ? p.accent.color.opacity(0.12) : .clear)
            )
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
    }
}
