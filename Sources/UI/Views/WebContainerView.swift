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
                view.frame = nsView.bounds
            }
            view.autoresizingMask = [.width, .height]
            let hidden = (tab.id != store.selectedTabID) || tab.didFail
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
        }
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
            for sub in subviews { sub.frame = bounds }
        }
        override func setFrameSize(_ newSize: NSSize) {
            super.setFrameSize(newSize)
            guard !freezeSubviewLayout else { return }
            for sub in subviews { sub.frame = bounds }
        }
    }
}
