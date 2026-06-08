import SwiftUI
import AppKit

/// Reveals the window titlebar when the cursor nears the top edge of the web
/// area: it slides the web card *down* (see `RootView`) so the genuine chrome
/// surface behind it shows through, and fades the traffic-light buttons in. The
/// card is moved with a transform, never resized — so the page never reflows.
///
/// This is a pure hover tracker: AppKit-hosted (like the sidebar peek) because
/// the CEF browser composites above SwiftUI and swallows mouse-moved events, so
/// `NSTrackingArea` is the only reliable way to detect the approach from any
/// direction. It captures no clicks (`hitTest` always passes through); the
/// traffic lights are real window buttons and stay clickable on their own.
struct TopChromeOverlay: NSViewRepresentable {
    @ObservedObject var store: BrowserStore
    var sidebarPosition: SidebarPosition

    func makeNSView(context: Context) -> TopChromeContainerView {
        let view = TopChromeContainerView()
        view.store = store
        view.sidebarPosition = sidebarPosition
        return view
    }

    func updateNSView(_ nsView: TopChromeContainerView, context: Context) {
        nsView.store = store
        nsView.sidebarPosition = sidebarPosition
        nsView.syncFromStore()
    }
}

final class TopChromeContainerView: NSView {
    weak var store: BrowserStore?
    var sidebarPosition: SidebarPosition = .right
    private var closeWork: DispatchWorkItem?
    private var hoverValidationTimer: Timer?
    private var appliedTrafficLightVisibility: Bool?

    /// Hover band at the very top that triggers the reveal when closed.
    private let edgeHeight: CGFloat = 18
    /// Keep-open band after the card has moved down. This includes the revealed
    /// titlebar plus a little hysteresis so the transition does not chatter.
    private let keepOpenHeight: CGFloat = 52
    /// Matches SidebarPeek's floating panel/handle band. While the real sidebar
    /// is hidden, this region belongs to sidebar peek, not top chrome.
    private let sidebarPeekExclusionWidth: CGFloat = 300
    /// How far the card drops — must match the offset applied in `RootView`.
    static let revealHeight: CGFloat = 28

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        wantsLayer = true
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override var isFlipped: Bool { true }

    private var isOpen: Bool { store?.topChromeRevealed ?? false }

    private func setOpen(_ open: Bool) {
        guard let store, store.topChromeRevealed != open else { return }
        withAnimation(Motion.snappy) { store.topChromeRevealed = open }
        applyTrafficLights(visible: open, animated: true)
        syncHoverValidationTimer()
    }

    func syncFromStore() {
        applyTrafficLights(visible: isOpen, animated: false)
        syncHoverValidationTimer()
        validateOpenStateAgainstMouse()
    }

    private func applyTrafficLights(visible: Bool, animated: Bool) {
        guard let window else { return }
        guard appliedTrafficLightVisibility != visible else { return }
        appliedTrafficLightVisibility = visible
        let buttons = [NSWindow.ButtonType.closeButton, .miniaturizeButton, .zoomButton]
            .compactMap { window.standardWindowButton($0) }

        closeWork?.cancel()
        closeWork = nil

        if visible {
            buttons.forEach {
                $0.isHidden = false
                $0.isEnabled = true
            }
        }

        let changes = {
            buttons.forEach { $0.animator().alphaValue = visible ? 1 : 0 }
        }

        let completion = { [weak self] in
            guard let self, !self.isOpen else { return }
            buttons.forEach {
                $0.isEnabled = false
                $0.isHidden = true
                $0.alphaValue = 0
            }
        }

        guard animated else {
            buttons.forEach {
                $0.alphaValue = visible ? 1 : 0
                $0.isEnabled = visible
                $0.isHidden = !visible
            }
            return
        }

        NSAnimationContext.runAnimationGroup { ctx in
            ctx.duration = 0.18
            changes()
        } completionHandler: {
            if !visible { completion() }
        }
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        syncFromStore() // start hidden
        DispatchQueue.main.async { [weak self] in self?.syncFromStore() }
        updateTrackingAreas()
    }

    deinit {
        hoverValidationTimer?.invalidate()
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
        if isOpen { scheduleClose() }
    }

    private func evaluate(_ event: NSEvent) {
        let point = convert(event.locationInWindow, from: nil)
        if isInSidebarPeekZone(point) {
            if isOpen { scheduleClose() }
            return
        }

        let y = point.y
        if isOpen {
            if y <= keepOpenHeight {
                closeWork?.cancel()
                closeWork = nil
            } else {
                scheduleClose()
            }
        } else if y <= edgeHeight {
            closeWork?.cancel()
            closeWork = nil
            setOpen(true)
        }
    }

    private func syncHoverValidationTimer() {
        if isOpen {
            guard hoverValidationTimer == nil else { return }
            let timer = Timer(timeInterval: 0.12, repeats: true) { [weak self] _ in
                self?.validateOpenStateAgainstMouse()
            }
            hoverValidationTimer = timer
            RunLoop.main.add(timer, forMode: .common)
        } else {
            hoverValidationTimer?.invalidate()
            hoverValidationTimer = nil
            closeWork?.cancel()
            closeWork = nil
        }
    }

    private func validateOpenStateAgainstMouse() {
        guard isOpen, let window else { return }
        let windowPoint = window.convertPoint(fromScreen: NSEvent.mouseLocation)
        let point = convert(windowPoint, from: nil)

        guard bounds.contains(point), !isInSidebarPeekZone(point), point.y <= keepOpenHeight else {
            scheduleClose()
            return
        }

        closeWork?.cancel()
        closeWork = nil
    }

    private func isInSidebarPeekZone(_ point: NSPoint) -> Bool {
        guard store?.sidebarVisible == false else { return false }
        switch sidebarPosition {
        case .left:
            return point.x <= sidebarPeekExclusionWidth
        case .right:
            return point.x >= bounds.maxX - sidebarPeekExclusionWidth
        }
    }

    private func scheduleClose() {
        guard closeWork == nil else { return }
        let work = DispatchWorkItem { [weak self] in
            self?.closeWork = nil
            self?.setOpen(false)
        }
        closeWork = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.18, execute: work)
    }

    // Pure tracker: never intercept clicks.
    override func hitTest(_ point: NSPoint) -> NSView? { nil }
}
