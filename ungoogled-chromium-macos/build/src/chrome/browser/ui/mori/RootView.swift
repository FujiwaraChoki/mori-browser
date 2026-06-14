import SwiftUI

/// The complete browser chrome: web content, optional AI panel, and the
/// user-positioned vertical tab sidebar (right by default).
struct RootView: View {
    @ObservedObject var store: BrowserStore
    @ObservedObject private var settings = BrowserSettings.shared
    @ObservedObject private var extensionStore = ExtensionStore.shared
    @Environment(\.colorScheme) private var systemScheme
    /// Live hover side while a sidebar tab is dragged over the web card.
    @State private var splitDropSide: BrowserStore.SplitSide?
    @State private var webCardWidth: CGFloat = 800

    private var gradientTheme: GradientTheme { settings.gradientTheme }

    private var scheme: ColorScheme {
        GradientEngine.effectiveScheme(
            for: gradientTheme,
            base: settings.theme.colorScheme ?? systemScheme
        )
    }

    private var palette: ThemePalette {
        ThemePalette.forScheme(scheme).applying(theme: gradientTheme, scheme: scheme)
    }

    var body: some View {
        let activeTab = store.selectedTab ?? store.tabs.first

        HStack(spacing: 0) {
            if store.sidebarVisible, settings.sidebarPosition == .left {
                Sidebar(store: store)
                    .transition(.move(edge: settings.sidebarPosition.edge))
            }

            // AI panel opens on the side opposite the tab sidebar: when the
            // sidebar sits on the right, the AI panel slides in from the left.
            if store.aiPanelVisible, settings.sidebarPosition == .right {
                AIPanel(store: store)
                    .transition(.move(edge: .leading).combined(with: .opacity))
            }

            // Web content column — the toolbar chrome plus a floating, rounded
            // "card" that encapsulates the live browser, Arc-style.
            VStack(spacing: 0) {
                WebTopStrip(tab: activeTab)
                webCard(activeTab: activeTab)
                    // Hovering the top edge slides the card down to reveal the
                    // chrome (and traffic lights) above it. Moved with a
                    // transform, so the web view is repositioned, never resized.
                    .offset(y: store.topChromeRevealed ? TopChromeContainerView.revealHeight : 0)
                    .animation(Motion.snappy, value: store.topChromeRevealed)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            // Hover the top edge to reveal the titlebar over the page.
            .overlay {
                TopChromeOverlay(store: store, sidebarPosition: settings.sidebarPosition)
            }

            // AI panel on the right, when the sidebar sits on the left.
            if store.aiPanelVisible, settings.sidebarPosition == .left {
                AIPanel(store: store)
                    .transition(.move(edge: .trailing).combined(with: .opacity))
            }

            if extensionStore.sidePanelExtensionID != nil {
                ExtensionSidePanel()
                    .transition(.move(edge: .trailing).combined(with: .opacity))
            }

            if store.sidebarVisible, settings.sidebarPosition == .right {
                Sidebar(store: store)
                    .transition(.move(edge: settings.sidebarPosition.edge))
            }
        }
        // Hover-to-peek sidebar — full-window overlay above the web view,
        // anchored to the selected sidebar edge, live only while hidden.
        .overlay {
            SidebarPeekOverlay(store: store, palette: palette, scheme: scheme,
                               gradientTheme: gradientTheme,
                               enabled: !store.sidebarVisible,
                               sidebarPosition: settings.sidebarPosition)
                .ignoresSafeArea()
        }
        // New-tab launcher (command palette) — full-window overlay so it centers
        // relative to the entire app window, not just the web card.
        .overlay {
            LauncherOverlay(store: store, palette: palette, scheme: scheme)
                .ignoresSafeArea()
        }
        // Transient notifications (link copied, etc.) — bottom-centered above
        // everything so they read clearly regardless of the active panel.
        .overlay {
            ToastOverlay(center: ToastCenter.shared)
                .ignoresSafeArea()
        }
        .environment(\.palette, palette)
        .preferredColorScheme(scheme)
        .background {
            // One unified chrome surface behind everything: the floating card's
            // inset gaps and the sidebar share this exact material + tint, so
            // there's no color step between them. A custom gradient theme washes
            // this surface with the picked colors (plus optional grain); with no
            // theme set it falls back to the plain sidebar tint.
            ZStack {
                VisualEffectBackground(material: .sidebar)
                if gradientTheme.isEmpty {
                    palette.sidebar.color.opacity(0.55)
                } else {
                    GradientEngine.chromeView(for: gradientTheme, scheme: scheme)
                        .opacity(gradientTheme.opacity)
                    if gradientTheme.texture > 0 {
                        GrainOverlay(amount: gradientTheme.texture)
                    }
                }
            }
            .ignoresSafeArea()
        }
        .ignoresSafeArea()
        .animation(Motion.reveal, value: store.aiPanelVisible)
        .animation(Motion.snappy, value: store.sidebarVisible)
        .animation(Motion.snappy, value: settings.sidebarPosition)
    }

    /// The browser, wrapped in a floating rounded card with a hairline border
    /// and a soft drop shadow, inset from the window edges so the chrome reads
    /// as a frame around the content (à la Arc).
    @ViewBuilder
    private func webCard(activeTab: BrowserTab?) -> some View {
        ZStack {
            // Card surface + shadow live on a real SwiftUI shape so the shadow
            // hugs the rounded corners (a clipped NSView can't cast one itself).
            RoundedRectangle(cornerRadius: Radius.window, style: .continuous)
                .fill(palette.card.color)
                .shadow(color: .black.opacity(scheme == .dark ? 0.40 : 0.10),
                        radius: 8, x: 0, y: 2)

            if let activeTab {
                ActiveWebContent(store: store,
                                 tab: activeTab,
                                 cornerRadius: Radius.window)
            }

            // Settings renders as a full page inside the card, on top of the
            // (suppressed) web content — not as a modal sheet.
            if store.settingsVisible {
                SettingsView(store: store)
                    .clipShape(RoundedRectangle(cornerRadius: Radius.window, style: .continuous))
            }

            // While the sidebar is being resized the CEF view is frozen (see
            // WebContainerView); cover it with the plain card surface so the
            // user sees a clean, smoothly-resizing card instead of the static,
            // clipped page underneath.
            if store.isResizingSidebar {
                RoundedRectangle(cornerRadius: Radius.window, style: .continuous)
                    .fill(palette.card.color)
            }
        }
        .overlay(alignment: .topTrailing) {
            if store.findBarVisible, let tab = activeTab {
                FindBar(store: store, tab: tab)
                    .transition(.move(edge: .top).combined(with: .opacity))
            }
        }
        .overlay(alignment: .bottom) {
            // Page-load indicator: a slim muted bar pinned to the bottom edge of
            // the page, clipped to the card's rounded corners.
            if let activeTab, activeTab.isLoading {
                LoadingBar()
                    .padding(.horizontal, Radius.window)
                    .padding(.bottom, 1)
                    .transition(.opacity)
                    .animation(Motion.state, value: activeTab.isLoading)
            }
        }
        .overlay(
            RoundedRectangle(cornerRadius: Radius.window, style: .continuous)
                .strokeBorder(palette.border.color.opacity(0.7), lineWidth: 1)
        )
        // Zen-style split: drag a sidebar tab over the card to split it.
        .overlay {
            if let side = splitDropSide {
                SplitDropPreview(side: side)
                    .clipShape(RoundedRectangle(cornerRadius: Radius.window,
                                                style: .continuous))
                    .allowsHitTesting(false)
                    .transition(.opacity)
            }
        }
        .overlay(alignment: store.splitSide == .left ? .topLeading : .topTrailing) {
            if store.splitTabID != nil {
                Button { store.closeSplit() } label: {
                    Icon(name: "xmark", size: 10, weight: .bold)
                        .foregroundStyle(palette.mutedForeground.color)
                        .frame(width: 22, height: 22)
                        .background(.regularMaterial, in: Circle())
                }
                .buttonStyle(.plain)
                .help("Close split")
                .padding(10)
            }
        }
        .background {
            GeometryReader { geo in
                Color.clear
                    .onAppear { webCardWidth = geo.size.width }
                    .onChange(of: geo.size.width) { _, w in webCardWidth = w }
            }
        }
        .onDrop(of: SidebarTabDrag.acceptedTypes,
                delegate: SplitDropDelegate(store: store,
                                            hoverSide: $splitDropSide,
                                            width: webCardWidth))
        .animation(Motion.snappy, value: splitDropSide != nil)
        .padding(.top, 4)
        .padding(.leading, 8)
        .padding(.trailing, 8)
        .padding(.bottom, 8)
    }
}

/// The drop-zone preview while dragging a tab over the web card: the half
/// being targeted is greyed/muted (the new tab lands there); the other half
/// keeps showing the existing page.
private struct SplitDropPreview: View {
    let side: BrowserStore.SplitSide
    @Environment(\.palette) private var p

    var body: some View {
        HStack(spacing: 0) {
            zone(active: side == .left)
            zone(active: side == .right)
        }
    }

    @ViewBuilder
    private func zone(active: Bool) -> some View {
        ZStack {
            if active {
                Rectangle().fill(Color.black.opacity(0.35))
                VStack(spacing: 8) {
                    Icon(name: "plus", size: 22, weight: .semibold)
                        .foregroundStyle(.white.opacity(0.9))
                    Text("Split here")
                        .font(Typography.ui(Typography.base, weight: .semibold))
                        .foregroundStyle(.white.opacity(0.9))
                }
            } else {
                Color.clear
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

/// Accepts a sidebar tab drag on the web card and turns it into a split.
private struct SplitDropDelegate: DropDelegate {
    let store: BrowserStore
    @Binding var hoverSide: BrowserStore.SplitSide?
    let width: CGFloat

    func validateDrop(info: DropInfo) -> Bool {
        info.hasItemsConforming(to: SidebarTabDrag.acceptedTypes)
    }

    func dropEntered(info: DropInfo) {
        hoverSide = side(for: info)
    }

    func dropUpdated(info: DropInfo) -> DropProposal? {
        hoverSide = side(for: info)
        return DropProposal(operation: .move)
    }

    func dropExited(info: DropInfo) {
        hoverSide = nil
    }

    func performDrop(info: DropInfo) -> Bool {
        let side = side(for: info)
        hoverSide = nil
        guard let provider = info.itemProviders(for: SidebarTabDrag.acceptedTypes).first else {
            return false
        }
        _ = provider.loadObject(ofClass: NSString.self) { object, _ in
            guard let string = object as? NSString,
                  let id = UUID(uuidString: string as String) else { return }
            DispatchQueue.main.async {
                store.splitWith(id, side: side)
            }
        }
        return true
    }

    private func side(for info: DropInfo) -> BrowserStore.SplitSide {
        // DropInfo.location is in the receiving view's coordinate space.
        info.location.x < width / 2 ? .left : .right
    }
}

private struct ActiveWebContent: View {
    @ObservedObject var store: BrowserStore
    @ObservedObject var tab: BrowserTab
    let cornerRadius: CGFloat

    var body: some View {
        WebContainerView(store: store, activeTab: tab, cornerRadius: cornerRadius)

        if tab.didFail {
            ErrorOverlay(tab: tab)
        }
    }
}

private struct GrainOverlay: View {
    let amount: Double

    var body: some View {
        ZStack {
            Color.white.opacity(0.035 * amount)
                .blendMode(.overlay)
            Color.black.opacity(0.025 * amount)
                .blendMode(.multiply)
        }
        .allowsHitTesting(false)
        .accessibilityHidden(true)
    }
}

/// Chrome's real extension side panel (an ExtensionViewHost owned by the
/// native bridge), framed by Mori's side panel chrome.
private struct ExtensionSidePanel: View {
    @ObservedObject private var extensions = ExtensionStore.shared
    @Environment(\.palette) private var p

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 8) {
                Icon(name: "sidebar.trailing", size: 15, weight: .regular)
                    .foregroundStyle(p.primary.color)
                Text(extensions.sidePanelTitle ?? "Extension")
                    .font(Typography.ui(15, weight: .semibold))
                    .foregroundStyle(p.foreground.color)
                    .lineLimit(1)
                Spacer()
                IconButton(systemName: "xmark", size: 28) {
                    extensions.closeSidePanel()
                }
            }
            .padding(.horizontal, 14)
            .frame(height: 48)

            Hairline().opacity(0.6)

            ExtensionSidePanelHost(extensionID: extensions.sidePanelExtensionID ?? "")
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .frame(width: 360)
        .background {
            ZStack {
                VisualEffectBackground(material: .menu)
                p.background.color.opacity(0.45)
            }
            .ignoresSafeArea()
        }
    }
}

/// Embeds the side panel host's native (WebContents) view. The bridge owns the
/// host's lifetime; this container only attaches/detaches the view.
private struct ExtensionSidePanelHost: NSViewRepresentable {
    let extensionID: String

    func makeNSView(context: Context) -> NSView {
        let container = NSView()
        attach(to: container)
        return container
    }

    func updateNSView(_ container: NSView, context: Context) {
        attach(to: container)
    }

    private func attach(to container: NSView) {
        guard let panelView = MoriChromeExtensions.sidePanelView() else {
            container.subviews.forEach { $0.removeFromSuperview() }
            return
        }
        if panelView.superview !== container {
            container.subviews.forEach { $0.removeFromSuperview() }
            panelView.frame = container.bounds
            panelView.autoresizingMask = [.width, .height]
            container.addSubview(panelView)
        }
    }
}

/// A lightweight failed-load overlay (e.g. no network / bad host).
private struct ErrorOverlay: View {
    @ObservedObject var tab: BrowserTab
    @Environment(\.palette) private var p

    var body: some View {
        VStack(spacing: 12) {
            Icon(name: "wifi.exclamationmark", size: 40, weight: .light)
                .foregroundStyle(p.mutedForeground.color)
            Text("This page couldn't load")
                .font(Typography.ui(15, weight: .medium))
                .foregroundStyle(p.foreground.color)
            Text(tab.urlString)
                .font(Typography.mono(12))
                .foregroundStyle(p.mutedForeground.color)
                .lineLimit(1)
                .truncationMode(.middle)
            Button {
                tab.reload()
            } label: {
                Text("Reload")
                    .font(Typography.ui(Typography.base))
                    .foregroundStyle(p.primaryForeground.color)
                    .padding(.horizontal, 14)
                    .padding(.vertical, 7)
                    .background(
                        RoundedRectangle(cornerRadius: Radius.button, style: .continuous)
                            .fill(p.primary.color)
                    )
            }
            .buttonStyle(.plain)
        }
        .padding(28)
        .frame(maxWidth: 360)
        .background(
            RoundedRectangle(cornerRadius: Radius.popover, style: .continuous)
                .fill(p.card.color)
        )
        .overlay(
            RoundedRectangle(cornerRadius: Radius.popover, style: .continuous)
                .strokeBorder(p.border.color.opacity(0.6), lineWidth: 1)
        )
    }
}
