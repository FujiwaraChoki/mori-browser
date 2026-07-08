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
    @ObservedObject private var archive = ArchiveStore.shared
    @Environment(\.palette) private var p

    enum Tab: String, CaseIterable {
        case history = "History", bookmarks = "Bookmarks", archive = "Archive"
    }
    @State private var tab: Tab = .history
    @State private var historySearch = ""
    @State private var bookmarkSearch = ""

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
            if tab == .archive && !archive.tabs.isEmpty {
                Button { archive.clear() } label: {
                    Text("Clear")
                        .font(Typography.ui(Typography.label, weight: .medium))
                        .foregroundStyle(p.mutedForeground.color)
                }
                .buttonStyle(.plain)
            }
        }
        .animation(Motion.state, value: tab)
        .padding(.horizontal, 12)
        .frame(height: 44)
    }

    @ViewBuilder
    private var content: some View {
        switch tab {
        case .history:
            VStack(spacing: 0) {
                searchField(text: $historySearch, placeholder: "Search History")
                if history.entries.isEmpty {
                    emptyState("clock", "No history yet")
                } else if historyResults.isEmpty {
                    emptyState("magnifyingglass", "No matching history")
                } else {
                    ScrollView {
                        LazyVStack(spacing: 1) {
                            ForEach(historyResults) { entry in
                                LibraryRow(title: entry.title,
                                           url: entry.url,
                                           trailing: libraryRelativeTime(entry.lastVisited)) { open(entry.url) }
                                    .contextMenu {
                                        Button("Remove", role: .destructive) { history.remove(entry) }
                                    }
                            }
                            if historySearch.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
                               history.entries.count > 200 {
                                Text("Showing the 200 most recent of \(history.entries.count) entries")
                                    .font(Typography.ui(Typography.small))
                                    .foregroundStyle(p.mutedForeground.color)
                                    .frame(maxWidth: .infinity)
                                    .padding(.vertical, 10)
                            }
                        }
                        .padding(8)
                    }
                }
            }
        case .bookmarks:
            VStack(spacing: 0) {
                searchField(text: $bookmarkSearch, placeholder: "Search Bookmarks")
                if bookmarks.bookmarks.isEmpty {
                    emptyState("star", "No bookmarks yet")
                } else if bookmarkResults.isEmpty {
                    emptyState("magnifyingglass", "No matching bookmarks")
                } else {
                    ScrollView {
                        LazyVStack(spacing: 1) {
                            ForEach(bookmarkResults) { mark in
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
        case .archive:
            if archive.tabs.isEmpty {
                emptyState("archivebox", "No archived tabs")
            } else {
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 1) {
                        ForEach(archiveGroups) { group in
                            archiveHeader(group.title)
                            ForEach(group.tabs) { archived in
                                LibraryRow(title: archived.title, url: archived.url) {
                                    store.restoreArchived(archived)
                                    isOpen = false
                                }
                                .contextMenu {
                                    Button("Reopen") {
                                        store.restoreArchived(archived)
                                        isOpen = false
                                    }
                                    Button("Remove", role: .destructive) { archive.remove(archived) }
                                }
                            }
                        }
                    }
                    .padding(8)
                }
            }
        }
    }

    private var historyResults: [HistoryEntry] {
        let q = historySearch.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !q.isEmpty else { return Array(history.entries.prefix(200)) }
        return history.suggestions(for: q, limit: history.entries.count)
    }

    private var bookmarkResults: [Bookmark] {
        let q = bookmarkSearch.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard !q.isEmpty else { return bookmarks.bookmarks }
        return bookmarks.bookmarks.filter {
            $0.title.lowercased().contains(q) || $0.url.lowercased().contains(q)
        }
    }

    private var archiveGroups: [ArchiveDateGroup] {
        ArchiveDateGroup.group(archive.tabs, calendar: .current, now: Date())
    }

    private func searchField(text: Binding<String>, placeholder: String) -> some View {
        HStack(spacing: 8) {
            Icon(name: "magnifyingglass", size: 13)
                .foregroundStyle(p.mutedForeground.color)
            TextField(placeholder, text: text)
                .textFieldStyle(.plain)
                .font(Typography.ui(Typography.base))
                .foregroundStyle(p.foreground.color)
        }
        .padding(.horizontal, 10)
        .frame(height: 30)
        .background(
            RoundedRectangle(cornerRadius: Radius.button, style: .continuous)
                .fill(p.foreground.color.opacity(0.06))
        )
        .padding(.horizontal, 10)
        .padding(.vertical, 8)
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

    private func archiveHeader(_ title: String) -> some View {
        Text(title.uppercased())
            .font(Typography.ui(Typography.small, weight: .medium))
            .foregroundStyle(p.mutedForeground.color)
            .tracking(0.4)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 9)
            .padding(.top, 9)
            .padding(.bottom, 4)
    }

    private func open(_ url: String) {
        store.navigate(url)
        isOpen = false
    }
}

/// Shared relative-time formatter for history rows (one alloc, not per-row).
private let libraryRelativeFormatter: RelativeDateTimeFormatter = {
    let f = RelativeDateTimeFormatter()
    f.unitsStyle = .abbreviated
    return f
}()

private func libraryRelativeTime(_ date: Date) -> String {
    libraryRelativeFormatter.localizedString(for: date, relativeTo: Date())
}

private struct ArchiveDateGroup: Identifiable {
    let id: String
    let title: String
    let tabs: [ArchivedTab]

    static func group(_ tabs: [ArchivedTab],
                      calendar: Calendar,
                      now: Date) -> [ArchiveDateGroup] {
        var today: [ArchivedTab] = []
        var yesterday: [ArchivedTab] = []
        var thisWeek: [ArchivedTab] = []
        var older: [ArchivedTab] = []
        let week = calendar.dateInterval(of: .weekOfYear, for: now)

        for tab in tabs {
            if calendar.isDateInToday(tab.archivedAt) {
                today.append(tab)
            } else if calendar.isDateInYesterday(tab.archivedAt) {
                yesterday.append(tab)
            } else if week?.contains(tab.archivedAt) == true {
                thisWeek.append(tab)
            } else {
                older.append(tab)
            }
        }

        return [
            ArchiveDateGroup(id: "today", title: "Today", tabs: today),
            ArchiveDateGroup(id: "yesterday", title: "Yesterday", tabs: yesterday),
            ArchiveDateGroup(id: "this-week", title: "This Week", tabs: thisWeek),
            ArchiveDateGroup(id: "older", title: "Older", tabs: older),
        ].filter { !$0.tabs.isEmpty }
    }
}

private struct LibraryRow: View {
    let title: String
    let url: String
    var trailing: String? = nil
    let action: () -> Void

    @Environment(\.palette) private var p
    @Environment(\.colorScheme) private var scheme
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            HStack(spacing: 10) {
                Favicon(icon: nil, page: url, size: 15)
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
                if let trailing {
                    Text(trailing)
                        .font(Typography.ui(Typography.small))
                        .foregroundStyle(p.mutedForeground.color)
                        .lineLimit(1)
                        .fixedSize()
                }
            }
            .padding(.horizontal, 9)
            .padding(.vertical, 6)
            .background(
                RoundedRectangle(cornerRadius: Radius.md, style: .continuous)
                    .fill(hovering ? TabSurface.hoverFill(scheme) : .clear)
            )
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .animation(Motion.state, value: hovering)
    }

    private var prettyURL: String {
        guard let u = URL(string: url) else { return url }
        return (u.host ?? "") + u.path
    }
}
