import SwiftUI

/// A fixed hairline strip atop the web content: its 4pt height plus the card's
/// 4pt top padding makes the top gap match the 8pt inset on the card's other
/// edges. Acts as the window drag area and shows the page-load progress bar. The
/// revealed titlebar (traffic lights + a slim bar) is handled separately by
/// `TopChromeOverlay`, which floats over the page rather than resizing it.
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
/// sweeps left→right; respects reduced-motion by holding still.
struct LoadingBar: View {
    @Environment(\.palette) private var p
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var phase: CGFloat = 0

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
                .onAppear {
                    guard !reduceMotion else { return }
                    withAnimation(.easeInOut(duration: 1.1).repeatForever(autoreverses: false)) {
                        phase = 1
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

    private var displayText: String { tab.displayURL }

    var body: some View {
        HStack(spacing: 7) {
            // The URL display doubles as the launch target. Extension items stay
            // outside the button so they remain independently clickable.
            Button(action: { store.presentLauncherForCurrentTab() }) {
                HStack(spacing: 7) {
                    Icon(name: secureGlyph, size: 13, weight: .regular)
                        .foregroundStyle(secureColor)

                    if displayText.isEmpty {
                        Text("Search or enter address")
                            .font(Typography.ui(Typography.base))
                            .foregroundStyle(p.mutedForeground.color.opacity(0.7))
                    } else {
                        Text(displayText)
                            .font(Typography.ui(Typography.base))
                            .foregroundStyle(p.foreground.color)
                            .lineLimit(1)
                            .truncationMode(.tail)
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
