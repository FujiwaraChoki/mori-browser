import SwiftUI

/// Toolbar entry point for downloads. Hidden until the first download starts,
/// then reveals a button that pulses while transfers are active and opens the
/// Downloads popover.
struct DownloadsButton: View {
    @ObservedObject var downloads: DownloadStore
    @Binding var isOpen: Bool
    @Environment(\.palette) private var p
    @State private var pulse = false

    var body: some View {
        Group {
            if downloads.items.isEmpty {
                EmptyView()
            } else {
                IconButton(systemName: glyph,
                           kind: isOpen ? .primary : .ghost,
                           size: 28) { isOpen.toggle() }
                    .opacity(downloads.hasActiveDownloads && pulse ? 0.55 : 1)
                    .animation(downloads.hasActiveDownloads
                               ? .easeInOut(duration: 0.7).repeatForever(autoreverses: true)
                               : .default,
                               value: pulse)
                    .onAppear { pulse = downloads.hasActiveDownloads }
                    .onChange(of: downloads.hasActiveDownloads) { _, active in
                        pulse = active
                    }
                    .popover(isPresented: $isOpen, arrowEdge: .bottom) {
                        DownloadsPanel(downloads: downloads)
                            .environment(\.palette, p)
                    }
                    .help("Downloads")
            }
        }
    }

    private var glyph: String {
        downloads.hasActiveDownloads ? "arrow.down.circle.fill" : "arrow.down.circle"
    }
}

/// The Downloads popover: a list of in-flight and finished downloads with
/// progress, reveal/open actions, and a clear button. Driven by `DownloadStore`.
struct DownloadsPanel: View {
    @ObservedObject var downloads: DownloadStore
    @Environment(\.palette) private var p

    var body: some View {
        VStack(spacing: 0) {
            header
            Hairline().opacity(0.6)
            if downloads.items.isEmpty {
                empty
            } else {
                list
            }
        }
        .frame(width: 360)
        .frame(maxHeight: 420)
        .background(p.popover.color)
    }

    private var header: some View {
        HStack(spacing: 8) {
            Text("Downloads")
                .font(Typography.ui(14, weight: .semibold))
                .foregroundStyle(p.foreground.color)
            Spacer()
            Button { downloads.showDefaultFolder() } label: {
                Text("Folder")
                    .font(Typography.ui(Typography.label, weight: .medium))
                    .foregroundStyle(p.mutedForeground.color)
            }
            .buttonStyle(.plain)
            if downloads.items.contains(where: { $0.isComplete || $0.isCanceled }) {
                Button { downloads.clearFinished() } label: {
                    Text("Clear")
                        .font(Typography.ui(Typography.label, weight: .medium))
                        .foregroundStyle(p.mutedForeground.color)
                }
                .buttonStyle(.plain)
            }
        }
        .padding(.horizontal, 14)
        .frame(height: 44)
    }

    private var empty: some View {
        VStack(spacing: 8) {
            Icon(name: "tray.and.arrow.down", size: 30, weight: .light)
                .foregroundStyle(p.mutedForeground.color)
            Text("No downloads yet")
                .font(Typography.ui(Typography.base))
                .foregroundStyle(p.mutedForeground.color)
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 40)
    }

    private var list: some View {
        ScrollView {
            LazyVStack(spacing: 2) {
                ForEach(downloads.items) { item in
                    DownloadRow(item: item, downloads: downloads)
                }
            }
            .padding(8)
        }
    }
}

private struct DownloadRow: View {
    let item: DownloadItem
    @ObservedObject var downloads: DownloadStore
    @Environment(\.palette) private var p
    @State private var hovering = false

    var body: some View {
        HStack(spacing: 11) {
            Icon(name: icon, size: 20, weight: .regular)
                .foregroundStyle(item.isComplete ? p.primary.color : p.mutedForeground.color)
                .frame(width: 22)

            VStack(alignment: .leading, spacing: 3) {
                Text(item.displayName)
                    .font(Typography.ui(Typography.base, weight: .medium))
                    .foregroundStyle(p.foreground.color)
                    .lineLimit(1)
                    .truncationMode(.middle)

                if item.isInProgress && !item.isComplete && !item.isCanceled {
                    ProgressView(value: min(max(item.fractionComplete, 0), 1))
                        .progressViewStyle(.linear)
                        .tint(p.primary.color)
                        .frame(height: 4)
                }

                Text(item.statusText)
                    .font(Typography.ui(Typography.small))
                    .foregroundStyle(p.mutedForeground.color)
                    .lineLimit(1)
            }

            Spacer(minLength: 6)

            if item.isInProgress && !item.isComplete && !item.isCanceled {
                Button { downloads.cancel(item) } label: {
                    Icon(name: "xmark", size: 14, weight: .semibold)
                        .foregroundStyle(p.mutedForeground.color)
                }
                .buttonStyle(.plain)
                .help("Cancel download")
                .opacity(hovering ? 1 : 0)
            } else if item.isComplete {
                Button { downloads.reveal(item) } label: {
                    Icon(name: "magnifyingglass", size: 14)
                        .foregroundStyle(p.mutedForeground.color)
                }
                .buttonStyle(.plain)
                .help("Show in Finder")
                .opacity(hovering ? 1 : 0)
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 8)
        .background(
            RoundedRectangle(cornerRadius: Radius.md, style: .continuous)
                .fill(hovering ? p.foreground.color.opacity(0.05) : .clear)
        )
        .contentShape(Rectangle())
        .onHover { hovering = $0 }
        .onTapGesture(count: 2) { downloads.open(item) }
    }

    private var icon: String {
        if item.isCanceled { return "xmark.circle" }
        if item.isComplete { return "doc.fill" }
        return "arrow.down.circle"
    }
}
