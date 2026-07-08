import SwiftUI

/// A fixed hairline strip atop the web content: its 4pt height plus the card's
/// 4pt top padding makes the top gap match the 8pt inset on the card's other
/// edges. Acts as the window drag area and shows the page-load progress bar. The
/// page-load indicator now lives inside the web card rather than in this strip.
struct WebTopStrip: View {
    var tab: BrowserTab?

    var body: some View {
        Color.clear
            .frame(maxWidth: .infinity)
            .frame(height: 4)
        .background {
            // Transparent: inherits the unified chrome surface set on the root,
            // so it's the exact same color as the sidebar (no seam).
            WindowDragArea()
                .ignoresSafeArea()
        }
        // The page-load indicator now lives as a muted bar pinned to the bottom
        // edge of the web card (see `RootView.webCard`).
    }
}

/// A slim indeterminate progress bar shown while a page loads. A muted segment
/// sweeps left→right; under reduced-motion it holds still and instead breathes
/// its opacity so it still reads as active "loading" rather than a dead line.
struct LoadingBar: View {
    @Environment(\.palette) private var p
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var phase: CGFloat = 0
    @State private var pulsing = false

    var body: some View {
        GeometryReader { geo in
            let w = geo.size.width
            let segment = max(120, w * 0.28)
            let tint = p.mutedForeground.color
            Capsule()
                .fill(
                    LinearGradient(
                        colors: [tint.opacity(0), tint.opacity(0.7), tint.opacity(0)],
                        startPoint: .leading, endPoint: .trailing
                    )
                )
                .frame(width: segment, height: 2)
                .offset(x: reduceMotion ? (w - segment) / 2 : phase * (w + segment) - segment)
                .opacity(reduceMotion ? (pulsing ? 1 : 0.35) : 1)
                .onAppear {
                    if reduceMotion {
                        withAnimation(Motion.pulse) { pulsing = true }
                    } else {
                        withAnimation(.easeInOut(duration: 1.1).repeatForever(autoreverses: false)) {
                            phase = 1
                        }
                    }
                }
        }
        .frame(height: 2)
    }
}

/// The address bar. Displays the current page URL; tapping it (or ⌘L) opens the
/// launcher seeded with that URL, so editing the address and searching share the
/// same command palette rather than an inline text field.
struct Omnibox: View {
    @ObservedObject var store: BrowserStore
    @ObservedObject var tab: BrowserTab
    @ObservedObject private var extensions = ExtensionStore.shared

    @Environment(\.palette) private var p

    private var displayText: String {
        guard !tab.displayURL.isEmpty else { return "" }
        guard let components = URLComponents(string: tab.displayURL),
              let host = components.host,
              !host.isEmpty else {
            return tab.displayURL
        }
        if let port = components.port {
            return "\(host):\(port)"
        }
        return host
    }

    var body: some View {
        HStack(spacing: 7) {
            SiteInfoButton(store: store, tab: tab)

            // The URL display doubles as the launch target. Extension items and
            // the site-info button stay outside so they remain independently
            // clickable.
            Button(action: { store.presentLauncherForCurrentTab() }) {
                HStack(spacing: 7) {
                    if displayText.isEmpty {
                        Text("Search or enter address")
                            .font(Typography.ui(Typography.base))
                            .foregroundStyle(p.mutedForeground.color.opacity(0.7))
                    } else {
                        Text(displayText)
                            .font(Typography.ui(Typography.base))
                            .foregroundStyle(p.foreground.color.opacity(0.78))
                            .lineLimit(1)
                            .truncationMode(.tail)
                            .help(tab.displayURL)
                    }

                    Spacer(minLength: 0)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)

            if let id = ExtensionStore.webStoreExtensionID(from: tab.urlString) {
                AddExtensionButton(installing: extensions.installingIDs.contains(id)) {
                    extensions.beginWebStoreInstall(extensionID: id)
                }
            }

            ExtensionToolbarItems(store: store)

            BookmarkStarButton(tab: tab)

            ReaderButton(store: store, tab: tab)

            if tab.isLoading {
                ProgressView().controlSize(.small).scaleEffect(0.55)
            }
        }
        .padding(.horizontal, 11)
        .frame(height: 30)
        .background {
            Color.clear.liquidGlass(cornerRadius: Radius.button)
        }
        .overlay(
            RoundedRectangle(cornerRadius: Radius.button, style: .continuous)
                .strokeBorder(p.border.color.opacity(0.35), lineWidth: 1)
        )
    }
}

private struct SiteInfoButton: View {
    @ObservedObject var store: BrowserStore
    @ObservedObject var tab: BrowserTab
    @ObservedObject private var bookmarks = BookmarkStore.shared
    @Environment(\.palette) private var p
    @State private var open = false

    @ViewBuilder private var leadingIcon: some View {
        if showsPageIcon {
            Favicon(icon: tab.faviconURL,
                    page: tab.urlString,
                    image: tab.faviconImage,
                    size: 15,
                    active: true)
        } else {
            Icon(name: secureGlyph, size: 13, weight: .regular)
                .foregroundStyle(secureColor)
        }
    }

    private var showsPageIcon: Bool {
        tab.urlString.hasPrefix("https://") || tab.urlString.hasPrefix("http://")
    }

    private var secureGlyph: String {
        if tab.urlString.hasPrefix("https") { return "lock.fill" }
        if tab.urlString.hasPrefix("http") { return "exclamationmark.triangle" }
        return "magnifyingglass"
    }

    private var secureColor: Color {
        if tab.urlString.hasPrefix("https") { return p.mutedForeground.color }
        if tab.urlString.hasPrefix("http") { return p.statusWarningFg.color }
        return p.mutedForeground.color
    }

    var body: some View {
        Button { open.toggle() } label: {
            leadingIcon
                .frame(width: 22, height: 22)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help("Site information")
        .popover(isPresented: $open, arrowEdge: .bottom) {
            siteInfoPanel
        }
    }

    private var hostText: String {
        URLComponents(string: tab.urlString)?.host ?? tab.displayURL
    }

    private var isSecure: Bool {
        tab.urlString.hasPrefix("https://")
    }

    private var isInsecureHTTP: Bool {
        tab.urlString.hasPrefix("http://")
    }

    private var statusTitle: String {
        if isSecure { return "Connection is secure" }
        if isInsecureHTTP { return "Connection is not secure" }
        if tab.urlString.hasPrefix("mori://") { return "Mori page" }
        if tab.urlString.isEmpty || tab.urlString == "about:blank" { return "New tab" }
        return "Page information"
    }

    @ViewBuilder
    private var siteInfoPanel: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 10) {
                Icon(name: secureGlyph, size: 17, weight: .semibold)
                    .foregroundStyle(secureColor)
                    .frame(width: 24, height: 24)
                VStack(alignment: .leading, spacing: 2) {
                    Text(statusTitle)
                        .font(Typography.ui(Typography.base, weight: .semibold))
                        .foregroundStyle(p.foreground.color)
                    Text(hostText.isEmpty ? "No site loaded" : hostText)
                        .font(Typography.ui(Typography.small))
                        .foregroundStyle(p.mutedForeground.color)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
                Spacer(minLength: 0)
            }
            .padding(12)

            Hairline().opacity(0.6)

            VStack(spacing: 2) {
                SiteInfoRow(icon: "link", title: "Copy Link") {
                    store.copyCurrentTabURL()
                    open = false
                }
                SiteInfoRow(icon: bookmarks.isBookmarked(tab.urlString) ? "star.fill" : "star",
                            title: bookmarks.isBookmarked(tab.urlString) ? "Remove Bookmark" : "Bookmark Page") {
                    store.toggleBookmark()
                    open = false
                }
                SiteInfoRow(icon: "square.and.arrow.up", title: "Share Page") {
                    store.share()
                    open = false
                }
                SiteInfoRow(icon: "qrcode", title: "Create QR Code") {
                    store.showQRCode()
                    open = false
                }
                SiteInfoRow(icon: "character.bubble", title: "Translate Page") {
                    store.translate()
                    open = false
                }
            }
            .padding(6)
        }
        .frame(width: 280)
        .background(p.popover.color)
    }
}

private struct SiteInfoRow: View {
    let icon: String
    let title: String
    let action: () -> Void
    @Environment(\.palette) private var p
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            HStack(spacing: 9) {
                Icon(name: icon, size: 14, weight: .medium)
                    .foregroundStyle(p.mutedForeground.color)
                    .frame(width: 18)
                Text(title)
                    .font(Typography.ui(Typography.base))
                    .foregroundStyle(p.foreground.color)
                Spacer()
            }
            .padding(.horizontal, 8)
            .frame(height: 30)
            .background(
                RoundedRectangle(cornerRadius: Radius.button, style: .continuous)
                    .fill(hovering ? p.foreground.color.opacity(0.07) : .clear)
            )
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .animation(Motion.state, value: hovering)
    }
}

/// The "Add to Mori" pill shown inside the omnibox on a Chrome Web Store
/// detail page. Tapping it downloads and installs the extension into Mori.
private struct AddExtensionButton: View {
    let installing: Bool
    let action: () -> Void
    @Environment(\.palette) private var p

    var body: some View {
        Button(action: action) {
            HStack(spacing: 4) {
                if installing {
                    ProgressView().controlSize(.small).scaleEffect(0.5)
                        .frame(width: 11, height: 11)
                } else {
                    Icon(name: "puzzlepiece.extension.fill", size: 11, weight: .semibold)
                }
                Text(installing ? "Adding…" : "Add to Mori")
                    .font(Typography.ui(Typography.small, weight: .medium))
            }
            .foregroundStyle(p.primaryForeground.color)
            .padding(.horizontal, 8)
            .frame(height: 22)
            .background(
                Capsule().fill(p.primary.color.opacity(installing ? 0.6 : 1))
            )
        }
        .buttonStyle(.plain)
        .disabled(installing)
        .help("Install this extension in Mori")
    }
}

/// A transparent AppKit view that lets you drag the window by the toolbar,
/// since the native titlebar is hidden.
struct WindowDragArea: NSViewRepresentable {
    func makeNSView(context: Context) -> NSView { DragView() }
    func updateNSView(_ nsView: NSView, context: Context) {}

    private final class DragView: NSView {
        override func mouseDown(with event: NSEvent) {
            window?.performDrag(with: event)
        }
        override var mouseDownCanMoveWindow: Bool { true }
    }
}
