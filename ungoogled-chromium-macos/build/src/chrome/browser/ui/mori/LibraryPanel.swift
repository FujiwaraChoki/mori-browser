import SwiftUI
import AppKit

/// Omnibox star that bookmarks (or un-bookmarks) the current page.
struct BookmarkStarButton: View {
    @ObservedObject var tab: BrowserTab
    @ObservedObject private var bookmarks = BookmarkStore.shared
    @Environment(\.palette) private var p

    private var saved: Bool { bookmarks.isBookmarked(tab.urlString) }

    var body: some View {
        Button {
            bookmarks.toggle(url: tab.urlString, title: tab.title)
        } label: {
            Icon(name: saved ? "star.fill" : "star", size: 15)
                .foregroundStyle(saved ? p.statusWarningFg.color : p.mutedForeground.color)
                .frame(width: 22, height: 22)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help(saved ? "Remove bookmark" : "Bookmark this page")
        .disabled(tab.urlString.isEmpty || tab.urlString == "about:blank")
    }
}

/// Toolbar entry point for the Library popover (history + bookmarks).
struct LibraryButton: View {
    @ObservedObject var store: BrowserStore
    @State private var open = false

    var body: some View {
        IconButton(systemName: "book", kind: open ? .primary : .ghost, size: 28) {
            open.toggle()
        }
        .help("History & Bookmarks")
        .popover(isPresented: $open, arrowEdge: .bottom) {
            LibraryPanel(store: store, isOpen: $open)
        }
    }
}

struct LibraryPanel: View {
    @ObservedObject var store: BrowserStore
    @Binding var isOpen: Bool
    @ObservedObject private var history = HistoryStore.shared
    @ObservedObject private var bookmarks = BookmarkStore.shared
    @Environment(\.palette) private var p

    enum Tab: String, CaseIterable { case history = "History", bookmarks = "Bookmarks" }
    @State private var tab: Tab = .history

    var body: some View {
        VStack(spacing: 0) {
            picker
            Hairline().opacity(0.6)
            content
            Hairline().opacity(0.6)
            footer
        }
        .frame(width: 380)
        .frame(maxHeight: 460)
        .background(p.popover.color)
    }

    private var footer: some View {
        HStack {
            Button(role: .destructive) { confirmClearAll() } label: {
                Label("Clear browsing data…", systemImage: "trash")
                    .font(Typography.ui(Typography.label, weight: .medium))
                    .foregroundStyle(p.destructive.color)
            }
            .buttonStyle(.plain)
            Spacer()
        }
        .padding(.horizontal, 12)
        .frame(height: 40)
    }

    private func confirmClearAll() {
        let alert = NSAlert()
        alert.messageText = "Clear browsing data?"
        alert.informativeText =
            "Choose what Mori should remove. Bookmarks are kept."
        alert.alertStyle = .warning
        alert.addButton(withTitle: "Clear")
        alert.addButton(withTitle: "Cancel")
        let history = clearDataCheckbox("Browsing history", checked: true)
        let cookies = clearDataCheckbox("Cookies and site sessions", checked: true)
        let cache = clearDataCheckbox("Cached files", checked: true)
        let downloads = clearDataCheckbox("Download list", checked: false)
        let stack = NSStackView(views: [history, cookies, cache, downloads])
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 6
        stack.edgeInsets = NSEdgeInsets(top: 4, left: 0, bottom: 0, right: 0)
        alert.accessoryView = stack
        if alert.runModal() == .alertFirstButtonReturn {
            store.clearBrowsingData(history: history.state == .on,
                                    cookies: cookies.state == .on,
                                    cache: cache.state == .on,
                                    downloads: downloads.state == .on)
            isOpen = false
        }
    }

    private func clearDataCheckbox(_ title: String, checked: Bool) -> NSButton {
        let button = NSButton(checkboxWithTitle: title, target: nil, action: nil)
        button.state = checked ? .on : .off
        return button
    }

    private var picker: some View {
        HStack(spacing: 4) {
            ForEach(Tab.allCases, id: \.self) { t in
                Button { tab = t } label: {
                    Text(t.rawValue)
                        .font(Typography.ui(Typography.base, weight: .medium))
                        .foregroundStyle(tab == t ? p.primaryForeground.color : p.foreground.color.opacity(0.8))
                        .padding(.horizontal, 12)
                        .frame(height: 26)
                        .background(
                            RoundedRectangle(cornerRadius: Radius.md, style: .continuous)
                                .fill(tab == t ? p.primary.color : .clear)
                        )
                }
                .buttonStyle(.plain)
            }
            Spacer()
            if tab == .history && !history.entries.isEmpty {
                Button { history.clear() } label: {
                    Text("Clear")
                        .font(Typography.ui(Typography.label, weight: .medium))
                        .foregroundStyle(p.mutedForeground.color)
                }
                .buttonStyle(.plain)
            }
        }
        .padding(.horizontal, 12)
        .frame(height: 44)
    }

    @ViewBuilder
    private var content: some View {
        switch tab {
        case .history:
            if history.entries.isEmpty {
                emptyState("clock", "No history yet")
            } else {
                ScrollView {
                    LazyVStack(spacing: 1) {
                        ForEach(history.entries.prefix(200)) { entry in
                            LibraryRow(title: entry.title, url: entry.url) { open(entry.url) }
                                .contextMenu {
                                    Button("Remove", role: .destructive) { history.remove(entry) }
                                }
                        }
                    }
                    .padding(8)
                }
            }
        case .bookmarks:
            if bookmarks.bookmarks.isEmpty {
                emptyState("star", "No bookmarks yet")
            } else {
                ScrollView {
                    LazyVStack(spacing: 1) {
                        ForEach(bookmarks.bookmarks) { mark in
                            LibraryRow(title: mark.title, url: mark.url) { open(mark.url) }
                                .contextMenu {
                                    Button("Remove", role: .destructive) { bookmarks.remove(mark) }
                                }
                        }
                    }
                    .padding(8)
                }
            }
        }
    }

    private func emptyState(_ symbol: String, _ text: String) -> some View {
        VStack(spacing: 8) {
            Icon(name: symbol, size: 28, weight: .light)
                .foregroundStyle(p.mutedForeground.color)
            Text(text)
                .font(Typography.ui(Typography.base))
                .foregroundStyle(p.mutedForeground.color)
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 44)
    }

    private func open(_ url: String) {
        store.navigate(url)
        isOpen = false
    }
}

private struct LibraryRow: View {
    let title: String
    let url: String
    let action: () -> Void

    @Environment(\.palette) private var p
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            HStack(spacing: 10) {
                Favicon(icon: faviconURL, page: url, size: 15)
                VStack(alignment: .leading, spacing: 1) {
                    Text(title.isEmpty ? url : title)
                        .font(Typography.ui(Typography.base))
                        .foregroundStyle(p.foreground.color)
                        .lineLimit(1)
                    Text(prettyURL)
                        .font(Typography.ui(Typography.small))
                        .foregroundStyle(p.mutedForeground.color)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 9)
            .padding(.vertical, 6)
            .background(
                RoundedRectangle(cornerRadius: Radius.md, style: .continuous)
                    .fill(hovering ? p.foreground.color.opacity(0.05) : .clear)
            )
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
    }

    private var faviconURL: String? {
        guard let host = URL(string: url)?.host else { return nil }
        return "https://www.google.com/s2/favicons?sz=32&domain=\(host)"
    }

    private var prettyURL: String {
        guard let u = URL(string: url) else { return url }
        return (u.host ?? "") + u.path
    }
}
