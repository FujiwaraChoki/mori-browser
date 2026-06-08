import SwiftUI

/// Toolbar entry point for downloads. Hidden until the first download starts,
/// then reveals a button wrapped in a live progress ring while transfers are
/// active. The Downloads popover opens on tap, and auto-opens the instant a
/// download finishes so the user never has to go hunting for the result.
struct DownloadsButton: View {
    @ObservedObject var downloads: DownloadStore
    @Binding var isOpen: Bool
    @Environment(\.palette) private var p
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var spin = false

    var body: some View {
        Group {
            if downloads.items.isEmpty {
                EmptyView()
            } else {
                ZStack {
                    if downloads.hasActiveDownloads {
                        progressRing
                    }
                    IconButton(systemName: glyph,
                               kind: isOpen ? .primary : .ghost,
                               size: 28) { isOpen.toggle() }
                }
                .popover(isPresented: $isOpen, arrowEdge: .bottom) {
                    DownloadsPanel(downloads: downloads)
                        .environment(\.palette, p)
                }
                .help(downloads.hasActiveDownloads ? "Downloading…" : "Downloads")
                .onChange(of: downloads.completionToken) { _, _ in
                    isOpen = true
                }
            }
        }
    }

    @ViewBuilder private var progressRing: some View {
        if downloads.hasIndeterminateActive {
            // Unknown size: a sweeping arc instead of a fill.
            Circle()
                .trim(from: 0, to: 0.7)
                .stroke(p.primary.color,
                        style: StrokeStyle(lineWidth: 2, lineCap: .round))
                .frame(width: 26, height: 26)
                .rotationEffect(.degrees(spin ? 360 : 0))
                .animation(reduceMotion ? nil
                           : .linear(duration: 0.9).repeatForever(autoreverses: false),
                           value: spin)
                .onAppear { spin = true }
        } else {
            ZStack {
                Circle()
                    .stroke(p.primary.color.opacity(0.18), lineWidth: 2)
                Circle()
                    .trim(from: 0, to: max(downloads.aggregateFraction, 0.03))
                    .stroke(p.primary.color,
                            style: StrokeStyle(lineWidth: 2, lineCap: .round))
                    .rotationEffect(.degrees(-90))
                    .animation(.easeOut(duration: 0.25), value: downloads.aggregateFraction)
            }
            .frame(width: 26, height: 26)
        }
    }

    private var glyph: String {
        downloads.hasActiveDownloads ? "arrow.down" : "arrow.down.circle"
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
