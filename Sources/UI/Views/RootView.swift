import SwiftUI

/// The complete browser chrome: web content, optional AI panel, and the
/// user-positioned vertical tab sidebar (right by default).
struct RootView: View {
    @ObservedObject var store: BrowserStore
    @ObservedObject private var settings = BrowserSettings.shared
    @Environment(\.colorScheme) private var systemScheme

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

            // AI panel.
            if store.aiPanelVisible {
                AIPanel(store: store)
                    .transition(.move(edge: .trailing).combined(with: .opacity))
            }

            if store.extensionSidePanelURL != nil {
                ExtensionSidePanel(store: store)
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
        .overlay(alignment: .topLeading) {
            ExtensionBackgroundRunners()
                .frame(width: 1, height: 1)
                .opacity(0.01)
                .allowsHitTesting(false)
                .accessibilityHidden(true)
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
        .animation(Motion.reveal, value: store.settingsVisible)
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
                    .transition(.opacity)
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
        .padding(.top, 4)
        .padding(.leading, 8)
        .padding(.trailing, 8)
        .padding(.bottom, 8)
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

private struct ExtensionBackgroundRunners: View {
    @ObservedObject private var extensions = ExtensionStore.shared

    var body: some View {
        ExtensionBackgroundRunnerHost(runners: extensions.backgroundRunners)
    }
}

private struct ExtensionBackgroundRunnerHost: NSViewRepresentable {
    let runners: [ExtensionStore.BackgroundRunner]

    func makeCoordinator() -> Coordinator {
        Coordinator()
    }

    func makeNSView(context: Context) -> NSView {
        let view = NSView(frame: NSRect(x: 0, y: 0, width: 1, height: 1))
        view.wantsLayer = true
        view.alphaValue = 0.01
        return view
    }

    func updateNSView(_ nsView: NSView, context: Context) {
        context.coordinator.sync(runners: runners, in: nsView)
    }

    static func dismantleNSView(_ nsView: NSView, coordinator: Coordinator) {
        coordinator.closeAll()
    }

    final class Coordinator {
        private struct RunnerView {
            let url: String
            let view: MoriBrowserView
        }

        private var views: [String: RunnerView] = [:]

        func sync(runners: [ExtensionStore.BackgroundRunner], in container: NSView) {
            let wanted = Set(runners.map(\.id))
            for id in Array(views.keys) where !wanted.contains(id) {
                views[id]?.view.closeBrowser()
                views[id]?.view.removeFromSuperview()
                views.removeValue(forKey: id)
            }

            for runner in runners {
                if let existing = views[runner.id] {
                    if existing.url != runner.url {
                        existing.view.loadURL(runner.url)
                        views[runner.id] = RunnerView(url: runner.url, view: existing.view)
                    }
                    continue
                }

                let view = MoriBrowserView(url: runner.url)
                view.frame = NSRect(x: 0, y: 0, width: 1, height: 1)
                view.autoresizingMask = []
                container.addSubview(view)
                views[runner.id] = RunnerView(url: runner.url, view: view)
            }
        }

        func closeAll() {
            for runner in views.values {
                runner.view.closeBrowser()
                runner.view.removeFromSuperview()
            }
            views.removeAll()
        }
    }
}

private struct ExtensionSidePanel: View {
    @ObservedObject var store: BrowserStore
    @Environment(\.palette) private var p

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 8) {
                Icon(name: "sidebar.trailing", size: 15, weight: .regular)
                    .foregroundStyle(p.primary.color)
                Text(store.extensionSidePanelTitle ?? "Extension")
                    .font(Typography.ui(15, weight: .semibold))
                    .foregroundStyle(p.foreground.color)
                    .lineLimit(1)
                Spacer()
                IconButton(systemName: "xmark", size: 28) {
                    store.closeExtensionSidePanel()
                }
            }
            .padding(.horizontal, 14)
            .frame(height: 48)

            Hairline().opacity(0.6)

            if let url = store.extensionSidePanelURL {
                ExtensionSidePanelBrowser(url: url)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
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

private struct ExtensionSidePanelBrowser: NSViewRepresentable {
    let url: String

    func makeNSView(context: Context) -> MoriBrowserView {
        let view = MoriBrowserView(url: url)
        view.setWebWindowVisible(true)
        return view
    }

    func updateNSView(_ view: MoriBrowserView, context: Context) {
        if view.currentURL != url {
            view.loadURL(url)
        }
        view.setWebWindowVisible(true)
    }

    static func dismantleNSView(_ view: MoriBrowserView, coordinator: ()) {
        view.closeBrowser()
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
