import SwiftUI
import AppKit

/// Hosts the live CEF browser views. All realized tabs stay mounted (so they
/// keep running like real background tabs); only the selected one is visible.
struct WebContainerView: NSViewRepresentable {
    @ObservedObject var store: BrowserStore
    @ObservedObject var activeTab: BrowserTab
    /// Corner radius applied to the container's layer so the live CEF content is
    /// clipped to the rounded "card" — SwiftUI `.clipShape` can't clip a hosted
    /// AppKit view, so the rounding has to happen on the layer itself.
    var cornerRadius: CGFloat = 0

    func makeNSView(context: Context) -> ContainerView {
        let view = ContainerView()
        view.applyCornerRadius(cornerRadius)
        return view
    }

    func updateNSView(_ nsView: ContainerView, context: Context) {
        let activeLoadFailed = activeTab.didFail
        nsView.applyCornerRadius(cornerRadius)

        // While the sidebar is being resized, freeze the CEF subviews so the
        // expensive async engine resize doesn't run every drag frame (the
        // source of the lag/flicker). On release this flips false and the
        // frame sync below resizes the engine once.
        let wasFrozen = nsView.freezeSubviewLayout
        nsView.freezeSubviewLayout = store.isResizingSidebar
        let justUnfroze = wasFrozen && !store.isResizingSidebar

        // Settings renders as a full page over the web card, so hide Chromium
        // while it is up. The launcher is hosted in an AppKit overlay above the
        // web view and should leave the current page visible behind its scrim.
        MoriBrowserView.setWebContentSuppressed(store.settingsVisible)

        // Make sure the selected tab is realized.
        store.selectedTab?.realize()

        let realizedTabs = store.tabs.filter { $0.hasRealized }
        let liveViews = realizedTabs.map { $0.browserView }

        // Remove views whose tabs are gone.
        for sub in nsView.subviews where !(liveViews.contains { $0 === sub }) {
            sub.removeFromSuperview()
        }

        // Split view: the selected tab and the split tab render side-by-side.
        let splitID = store.splitTabID
        nsView.splitLayout = nil
        if let splitID, let splitTab = realizedTabs.first(where: { $0.id == splitID }),
           let primary = realizedTabs.first(where: { $0.id == store.selectedTabID }),
           splitTab.id != primary.id {
            let left = store.splitSide == .left ? splitTab.browserView
                                                : primary.browserView
            let right = store.splitSide == .left ? primary.browserView
                                                 : splitTab.browserView
            nsView.splitLayout = (left: left, right: right)
        }

        // Add, position, and set visibility for current tabs.
        for tab in realizedTabs {
            let view = tab.browserView
            if view.superview !== nsView {
                view.removeFromSuperview()
                nsView.addSubview(view)
            }
            // Hold the engine's frame steady while resizing; sync it otherwise
            // (and force a one-shot sync on the frame we just unfroze).
            if !nsView.freezeSubviewLayout || justUnfroze {
                view.frame = nsView.frameForSubview(view)
            }
            view.autoresizingMask = nsView.splitLayout == nil ? [.width, .height] : []
            let visible = tab.id == store.selectedTabID || (splitID != nil && tab.id == splitID)
            let hidden = !visible || tab.didFail
            view.isHidden = hidden
            view.setWebWindowVisible(!hidden)
            // Drive Chromium's page-visibility so backgrounded tabs throttle and
            // (when enabled) auto-enter Picture-in-Picture on tab switch.
            view.setPageHidden(hidden)
        }

        // Keep the active browser keyboard-focused.
        if let active = store.selectedTab, active.hasRealized {
            active.browserView.isHidden = activeLoadFailed
            active.browserView.setWebWindowVisible(!activeLoadFailed)
            if store.shouldAutoFocusWebContent,
               !activeLoadFailed,
               !Self.windowHasTextInputFocus(nsView.window) {
                DispatchQueue.main.async { [weak browserView = active.browserView,
                                            weak store,
                                            weak nsView] in
                    guard let browserView,
                          let store,
                          store.shouldAutoFocusWebContent,
                          !browserView.isHidden,
                          !Self.windowHasTextInputFocus(nsView?.window)
                    else { return }
                    browserView.focusBrowser()
                }
            }
        }
    }

    private static func windowHasTextInputFocus(_ window: NSWindow?) -> Bool {
        guard let responder = window?.firstResponder else { return false }
        return responder is NSTextView || responder is NSTextField
    }

    /// Flipped container so child frames use top-left origin.
    final class ContainerView: NSView {
        override var isFlipped: Bool { true }

        /// When true, the hosted CEF subviews keep their current frame instead
        /// of tracking `bounds` — set during a sidebar resize drag so the engine
        /// isn't re-laid-out every frame. Also gates AppKit's mask-based
        /// autoresizing so a parent frame change can't resize them behind us.
        var freezeSubviewLayout = false {
            didSet { autoresizesSubviews = !freezeSubviewLayout }
        }

        /// Non-nil while a vertical split is active: the two panes, each laid
        /// out as half of the container with a thin gap between them.
        var splitLayout: (left: NSView, right: NSView)? {
            didSet { needsLayout = true }
        }

        static let splitGap: CGFloat = 8

        func frameForSubview(_ view: NSView) -> NSRect {
            guard let split = splitLayout else { return bounds }
            let half = (bounds.width - Self.splitGap) / 2
            if view === split.left {
                return NSRect(x: 0, y: 0, width: half, height: bounds.height)
            }
            if view === split.right {
                return NSRect(x: half + Self.splitGap, y: 0,
                              width: half, height: bounds.height)
            }
            return bounds
        }

        /// Round (and clip to) the layer so the hosted CEF subviews are masked
        /// to the card shape. `.continuous` matches SwiftUI's squircle corners.
        func applyCornerRadius(_ radius: CGFloat) {
            wantsLayer = true
            guard let layer else { return }
            if layer.cornerRadius != radius { layer.cornerRadius = radius }
            layer.cornerCurve = .continuous
            layer.masksToBounds = radius > 0
        }

        override func layout() {
            super.layout()
            guard !freezeSubviewLayout else { return }
            for sub in subviews { sub.frame = frameForSubview(sub) }
        }
        override func setFrameSize(_ newSize: NSSize) {
            super.setFrameSize(newSize)
            guard !freezeSubviewLayout else { return }
            for sub in subviews { sub.frame = frameForSubview(sub) }
        }
    }
}
