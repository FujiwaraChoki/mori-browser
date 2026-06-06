import SwiftUI
import AppKit

/// Arc-style "peek" sidebar. When the sidebar is toggled closed, hovering the
/// selected window edge slides the full sidebar in as a floating overlay (not
/// docked — the web layout doesn't reflow). It stays open until the cursor
/// leaves it.
///
/// This has to be AppKit-driven: the live CEF browser is a hosted `NSView` that
/// composites *above* SwiftUI content and swallows its mouse events, so a plain
/// SwiftUI `.overlay` would render behind the page and never receive hover. We
/// host the peek in an `NSView` placed above everything and override `hitTest`
/// to stay click-through except on the open panel. Hover is read continuously
/// from a single full-bounds tracking area (no fragile area swapping).
struct SidebarPeekOverlay: NSViewRepresentable {
    @ObservedObject var store: BrowserStore
    var palette: ThemePalette
    var scheme: ColorScheme
    var gradientTheme: GradientTheme
    /// Active only while the sidebar is hidden; otherwise fully pass-through.
    var enabled: Bool
    var sidebarPosition: SidebarPosition

    func makeNSView(context: Context) -> PeekContainerView {
        let view = PeekContainerView()
        view.update(store: store, palette: palette, scheme: scheme, gradientTheme: gradientTheme,
                    enabled: enabled, sidebarPosition: sidebarPosition)
        return view
    }

    func updateNSView(_ nsView: PeekContainerView, context: Context) {
        nsView.update(store: store, palette: palette, scheme: scheme, gradientTheme: gradientTheme,
                      enabled: enabled, sidebarPosition: sidebarPosition)
    }
}

/// Drives the SwiftUI peek's open/closed state from the AppKit hover logic.
final class PeekModel: ObservableObject {
    @Published var isOpen = false
    @Published var enabled = false
}

/// Hosts the peek UI above the web view and gates interaction via `hitTest`.
final class PeekContainerView: NSView {
    private let model = PeekModel()
    private var hosting: NSHostingView<AnyView>?
    private var closeWork: DispatchWorkItem?

    /// Cursor band at the selected edge that triggers the peek when closed.
    private let edgeWidth: CGFloat = 12
    /// Interactive / keep-open band (the floating card + handle + margins).
    private let panelBand: CGFloat = 300

    private weak var store: BrowserStore?
    private var palette: ThemePalette = .light
    private var scheme: ColorScheme = .light
    private var gradientTheme: GradientTheme = .none
    private var sidebarPosition: SidebarPosition = .right

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

    func update(store: BrowserStore, palette: ThemePalette, scheme: ColorScheme,
                gradientTheme: GradientTheme,
                enabled: Bool, sidebarPosition: SidebarPosition) {
        self.store = store
        self.palette = palette
        self.scheme = scheme
        self.gradientTheme = gradientTheme
        self.sidebarPosition = sidebarPosition
        let appearance = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua)
        self.appearance = appearance
        hosting?.appearance = appearance
        if model.enabled != enabled {
            model.enabled = enabled
            if !enabled { setOpen(false) }
        }
        rebuild()
    }

    private func rebuild() {
        guard let store else { return }
        hosting?.rootView = AnyView(
            PeekUI(store: store, model: model, palette: palette, scheme: scheme,
                   gradientTheme: gradientTheme,
                   sidebarPosition: sidebarPosition)
                .environment(\.palette, palette)
                .preferredColorScheme(scheme)
        )
    }

    private func setOpen(_ open: Bool) {
        guard model.isOpen != open else { return }
        model.isOpen = open
    }

    // MARK: Hover tracking — one persistent area, position read continuously.

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        trackingAreas.forEach(removeTrackingArea)
        addTrackingArea(NSTrackingArea(
            rect: .zero,
            options: [.mouseMoved, .mouseEnteredAndExited, .activeAlways, .inVisibleRect],
            owner: self
        ))
    }

    override func mouseMoved(with event: NSEvent) { evaluate(event) }
    override func mouseEntered(with event: NSEvent) { evaluate(event) }

    override func mouseExited(with event: NSEvent) {
        // Cursor left the window entirely.
        if model.isOpen { scheduleClose() }
    }

    private func evaluate(_ event: NSEvent) {
        guard model.enabled else { return }
        let x = convert(event.locationInWindow, from: nil).x
        if model.isOpen {
            if isInPanelBand(x) {
                closeWork?.cancel()
            } else {
                scheduleClose()
            }
        } else if isInEdgeBand(x) {
            closeWork?.cancel()
            setOpen(true)
        }
    }

    private func isInEdgeBand(_ x: CGFloat) -> Bool {
        switch sidebarPosition {
        case .left: return x <= edgeWidth
        case .right: return x >= bounds.maxX - edgeWidth
        }
    }

    private func isInPanelBand(_ x: CGFloat) -> Bool {
        switch sidebarPosition {
        case .left: return x <= panelBand
        case .right: return x >= bounds.maxX - panelBand
        }
    }

    private func scheduleClose() {
        guard closeWork == nil else { return }
        let work = DispatchWorkItem { [weak self] in
            self?.closeWork = nil
            self?.setOpen(false)
        }
        closeWork = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.16, execute: work)
    }

    // MARK: Click-through gating

    override func hitTest(_ point: NSPoint) -> NSView? {
        // Pass clicks through to the web view unless the panel is open and the
        // click lands on it.
        guard model.enabled, model.isOpen else { return nil }
        let local = convert(point, from: superview)
        return isInPanelBand(local.x) ? super.hitTest(point) : nil
    }
}

// MARK: - SwiftUI peek contents

private struct PeekUI: View {
    @ObservedObject var store: BrowserStore
    @ObservedObject var model: PeekModel
    var palette: ThemePalette
    var scheme: ColorScheme
    var gradientTheme: GradientTheme
    var sidebarPosition: SidebarPosition

    private let cardWidth: CGFloat = 256
    private let inset: CGFloat = 8

    private var isLeft: Bool { sidebarPosition == .left }

    var body: some View {
        ZStack(alignment: isLeft ? .leading : .trailing) {
            if model.enabled {
                EdgeHandle(open: model.isOpen, sidebarPosition: sidebarPosition)
                    .offset(x: handleOffset)

                panel
                    .offset(x: panelOffset)
                    .opacity(model.isOpen ? 1 : 0)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity,
               alignment: isLeft ? .leading : .trailing)
        // Extend past the title-bar safe-area inset so the card's top edge rises
        // to the window top (the 8pt vertical inset becomes the only top gap),
        // matching the web card rather than starting below the toolbar row.
        .ignoresSafeArea(.container, edges: .vertical)
        .animation(Motion.snappy, value: model.isOpen)
        .animation(Motion.snappy, value: model.enabled)
    }

    private var handleOffset: CGFloat {
        let resting: CGFloat = isLeft ? 10 : -10
        let open = cardWidth + inset + 8
        return model.isOpen ? (isLeft ? open : -open) : resting
    }

    private var panelOffset: CGFloat {
        guard !model.isOpen else { return 0 }
        let hidden = cardWidth + inset + 16
        return isLeft ? -hidden : hidden
    }

    /// The full sidebar, wrapped as a floating, inset card spanning the window.
    private var panel: some View {
        Sidebar(store: store)
            .frame(width: cardWidth)
            .frame(maxHeight: .infinity)
            .background {
                PeekPanelBackground(palette: palette, scheme: scheme, gradientTheme: gradientTheme)
            }
            .clipShape(RoundedRectangle(cornerRadius: Radius.window, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: Radius.window, style: .continuous)
                    .strokeBorder(palette.sidebarBorder.color.opacity(scheme == .dark ? 0.70 : 0.62),
                                  lineWidth: 1)
            )
            .shadow(color: .black.opacity(scheme == .dark ? 0.46 : 0.16),
                    radius: 22, x: isLeft ? 7 : -7, y: 5)
            .padding(.vertical, inset)
            .padding(isLeft ? .leading : .trailing, inset)
    }
}

private struct PeekPanelBackground: View {
    var palette: ThemePalette
    var scheme: ColorScheme
    var gradientTheme: GradientTheme

    var body: some View {
        ZStack {
            VisualEffectBackground(material: .sidebar)
            if gradientTheme.isEmpty {
                palette.sidebar.color.opacity(scheme == .dark ? 0.82 : 0.86)
            } else {
                GradientEngine.chromeView(for: gradientTheme, scheme: scheme)
                    .opacity(gradientTheme.opacity)
                palette.sidebar.color.opacity(scheme == .dark ? 0.22 : 0.18)
                if gradientTheme.texture > 0 {
                    PeekGrainOverlay(amount: gradientTheme.texture)
                }
            }
        }
    }
}

private struct PeekGrainOverlay: View {
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

/// The resting hint: a faint translucent vertical line that morphs into a
/// left-pointing chevron when the peek engages, sitting just left of the
/// incoming sidebar card.
private struct EdgeHandle: View {
    var open: Bool
    var sidebarPosition: SidebarPosition
    @Environment(\.palette) private var p

    var body: some View {
        ZStack {
            Capsule()
                .fill(p.sidebarForeground.color.opacity(0.24))
                .frame(width: 4, height: 40)
                .opacity(open ? 0 : 1)
                .scaleEffect(y: open ? 0.4 : 1, anchor: .center)

            ZStack {
                Circle()
                    .fill(p.sidebar.color.opacity(0.58))
                    .overlay(Circle().strokeBorder(p.sidebarBorder.color.opacity(0.55), lineWidth: 1))
                Icon(name: sidebarPosition == .left ? "chevron.right" : "chevron.left",
                     size: 13, weight: .semibold)
                    .foregroundStyle(p.sidebarForeground.color.opacity(0.76))
            }
            .frame(width: 24, height: 24)
            .opacity(open ? 1 : 0)
            .scaleEffect(open ? 1 : 0.58)
        }
        .frame(width: 18, height: 44)
        .allowsHitTesting(false)
    }
}
