import SwiftUI
import AppKit

/// Bridge object the ObjC++ AppDelegate calls to build and own the SwiftUI
/// chrome. Holds the single shared BrowserStore for the window.
@objc(MoriRoot)
final class MoriRoot: NSObject {
    /// Retained for the app lifetime so the store/tabs aren't deallocated.
    private static var shared: MoriRoot?
    /// The SwiftUI chrome's backing view. Held weakly (the window owns it) so a
    /// keyboard-driven toggle can force an immediate layout/display pass — see
    /// flushChrome().
    private static weak var chromeView: NSView?

    let store = BrowserStore()

    @objc static func makeRootViewController() -> NSViewController {
        let root = MoriRoot()
        shared = root

        let hosting = NSHostingController(rootView: RootView(store: root.store))
        hosting.view.frame = NSRect(x: 0, y: 0, width: 1280, height: 820)
        chromeView = hosting.view
        return hosting
    }

    /// Force the SwiftUI chrome to lay out and draw *now*.
    ///
    /// Keyboard shortcuts mutate the store from outside SwiftUI's own event
    /// handling — the AppKit event monitor (which consumes the event) and
    /// Chromium's `PreHandleKeyboardEvent`. Under Chromium's custom Mac message
    /// pump the resulting `@Published` change is scheduled but not committed
    /// until some later, unrelated event pumps the run loop, so the sidebar
    /// "only toggles when you take an action." Driving layout/display here
    /// commits it immediately, matching the click-the-button path that works.
    private static func flushChrome() {
        flushChromeNow()
        // The synchronous flush above renders the *pre-mutation* tree: SwiftUI
        // commits a keyboard-driven `@Published` change on the run loop's
        // BeforeWaiting observer, which hasn't fired yet inside this call stack.
        // Hop to the next run-loop tick — by then the commit has landed — and
        // redraw immediately, so the toggle's first frame shows up a frame
        // sooner than the timer pulse below would deliver it.
        DispatchQueue.main.async { flushChromeNow() }
        // A keyboard shortcut mutates the store from outside SwiftUI's own event
        // handling, so SwiftUI commits the change and advances its animation off
        // the main run loop's update observer + animation timer — both of which
        // only fire when the loop iterates. Under Chromium's custom Mac message
        // pump the loop parks immediately after consuming the key event, so the
        // toggle and its 0.15–0.25s animation stall until some *later* event
        // pumps the loop again ("needs two presses"). A single forced redraw only
        // lands the animation's first frame (≈ no visible change) and then stalls
        // again. Instead, pulse a redraw across the animation window: each pulse
        // wakes the loop and re-evaluates the in-progress animation against the
        // current clock, so the toggle lands and animates through on the first
        // press, matching the click-the-button path. Overlapping pulses (held
        // repeats) coalesce into one bounded loop via the shared deadline.
        flushPulseDeadline = ProcessInfo.processInfo.systemUptime + flushSettleDuration
        scheduleFlushPulseIfNeeded()
    }

    /// ~0.25s Motion.reveal (the longest toggle animation) plus a small buffer.
    private static let flushSettleDuration: TimeInterval = 0.35
    private static var flushPulseDeadline: TimeInterval = 0
    private static var flushPulseScheduled = false

    private static func scheduleFlushPulseIfNeeded() {
        guard !flushPulseScheduled else { return }
        guard ProcessInfo.processInfo.systemUptime < flushPulseDeadline else {
            // The pulse window elapsed. If the main thread was starved (heavy
            // page JS/layout) the in-flight pulses may have been delayed past
            // the deadline; do one final redraw so the *settled*, end-of-
            // animation state is guaranteed on screen instead of a stalled
            // mid-animation frame, then stop pulsing.
            flushChromeNow()
            return
        }
        flushPulseScheduled = true
        // ~60fps; libdispatch wakes the parked run loop at each deadline.
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.0 / 60.0) {
            flushPulseScheduled = false
            flushChromeNow()
            scheduleFlushPulseIfNeeded()
        }
    }

    private static func flushChromeNow() {
        // Normally the hosting view set in makeRootViewController; if that weak
        // ref ever dies, fall back to the live window's content view so a
        // keyboard toggle still forces a repaint instead of silently no-op'ing.
        guard let view = chromeView
            ?? (NSApp.keyWindow ?? NSApp.mainWindow)?.contentView else { return }
        let contentView = view.window?.contentView

        contentView?.needsLayout = true
        contentView?.needsDisplay = true
        contentView?.layoutSubtreeIfNeeded()

        view.needsLayout = true
        view.needsDisplay = true
        view.layoutSubtreeIfNeeded()
        contentView?.displayIfNeeded()
        view.displayIfNeeded()
        view.window?.viewsNeedDisplay = true
        view.window?.displayIfNeeded()
        pumpAppKitForChromeUpdate()
    }

    /// Post a no-op AppKit event so NSApplication runs a real event cycle.
    ///
    /// The display calls above only *redraw the already-committed view tree*.
    /// A keyboard shortcut, though, mutates an `@Published` value from outside
    /// SwiftUI's own event handling, and SwiftUI applies that change (and
    /// advances its `withAnimation`) from a run-loop observer that, under
    /// Chromium's custom Mac message pump, only fires when AppKit actually
    /// processes an event. `displayIfNeeded` does *not* count — which is why a
    /// ⌘S sidebar toggle would land in the store yet not appear until some
    /// unrelated keypress or mouse-move pumped the loop ("works only after you
    /// take another action"). Posting a synthetic application-defined event
    /// reproduces that wake-up deterministically: NSApp dequeues it, runs the
    /// cycle, SwiftUI reconciles, and the toggle shows on the first press.
    /// The event is inert — the app-level key/mouse monitor filters it out,
    /// and no view acts on `applicationDefined` — so it has no side effects.
    private static func pumpAppKitForChromeUpdate() {
        guard let event = NSEvent.otherEvent(with: .applicationDefined,
                                             location: .zero,
                                             modifierFlags: [],
                                             timestamp: ProcessInfo.processInfo.systemUptime,
                                             windowNumber: 0,
                                             context: nil,
                                             subtype: 0,
                                             data1: 0,
                                             data2: 0) else { return }
        NSApp.postEvent(event, atStart: false)
    }

    @objc static func prepareForTermination() {
        shared?.store.prepareForTermination()
    }

    @objc static func shouldAutoFocusWebContent() -> Bool {
        shared?.store.shouldAutoFocusWebContent ?? false
    }

    @objc static func handleShortcutEvent(_ event: NSEvent) -> Bool {
        guard Thread.isMainThread else {
            var handled = false
            DispatchQueue.main.sync {
                handled = handleShortcutEvent(event)
            }
            return handled
        }
        guard let store = shared?.store else { return false }
        let handled = MoriCommands.handle(event, store: store)
        if handled { flushChrome() }
        return handled
    }

    @objc static func releaseShortcutEvent(_ event: NSEvent) {
        guard Thread.isMainThread else {
            DispatchQueue.main.sync {
                releaseShortcutEvent(event)
            }
            return
        }
        MoriCommands.release(event)
    }

    @objc(isReservedShortcutKeyEquivalent:modifierMask:)
    static func isReservedShortcut(keyEquivalent: String, modifierMask: UInt) -> Bool {
        MoriCommands.reservesChromiumShortcut(keyEquivalent: keyEquivalent,
                                              modifierMask: modifierMask)
    }

    @objc(handleShortcutWithKeyCode:charactersIgnoringModifiers:modifierMask:)
    static func handleShortcut(keyCode: UInt16,
                               charactersIgnoringModifiers: String?,
                               modifierMask: UInt) -> Bool {
        handleShortcut(keyCode: keyCode,
                       charactersIgnoringModifiers: charactersIgnoringModifiers,
                       modifierMask: modifierMask,
                       isRepeat: false)
    }

    @objc(handleShortcutWithKeyCode:charactersIgnoringModifiers:modifierMask:isRepeat:)
    static func handleShortcut(keyCode: UInt16,
                               charactersIgnoringModifiers: String?,
                               modifierMask: UInt,
                               isRepeat: Bool) -> Bool {
        guard Thread.isMainThread else {
            var handled = false
            DispatchQueue.main.sync {
                handled = handleShortcut(keyCode: keyCode,
                                         charactersIgnoringModifiers: charactersIgnoringModifiers,
                                         modifierMask: modifierMask,
                                         isRepeat: isRepeat)
            }
            return handled
        }
        guard let store = shared?.store else { return false }
        let handled = MoriCommands.handle(keyCode: keyCode,
                                          charactersIgnoringModifiers: charactersIgnoringModifiers,
                                          modifierMask: modifierMask,
                                          isRepeat: isRepeat,
                                          store: store)
        if handled { flushChrome() }
        return handled
    }

    @objc(releaseShortcutWithKeyCode:charactersIgnoringModifiers:modifierMask:)
    static func releaseShortcut(keyCode: UInt16,
                                charactersIgnoringModifiers: String?,
                                modifierMask: UInt) {
        guard Thread.isMainThread else {
            DispatchQueue.main.sync {
                releaseShortcut(keyCode: keyCode,
                                charactersIgnoringModifiers: charactersIgnoringModifiers,
                                modifierMask: modifierMask)
            }
            return
        }
        MoriCommands.release(keyCode: keyCode,
                             charactersIgnoringModifiers: charactersIgnoringModifiers,
                             modifierMask: modifierMask)
    }

    // Menu-driven actions (called from the AppKit menu bar).
    // ⌘T / File ▸ New Tab toggles the launcher (command palette) rather than
    // silently spawning a blank tab.
    @objc static func newTab() { shared?.store.toggleLauncher() }
    @objc static func dismissLauncherIfVisible() -> Bool {
        guard let store = shared?.store, store.launcherVisible else { return false }
        store.dismissLauncher()
        return true
    }
    @objc(openNewTabWithURL:)
    static func openNewTab(url: String) {
        shared?.store.newTab(url: url.isEmpty ? "about:blank" : url, select: true)
    }
    @objc static func closeCurrentTab() {
        if let id = shared?.store.selectedTabID { shared?.store.closeTab(id) }
    }
    @objc static func reopenClosedTab() { shared?.store.reopenClosedTab() }
    @objc static func reload() { shared?.store.reload() }
    @objc static func forceReload() { shared?.store.reloadIgnoringCache() }
    @objc static func stop() { shared?.store.stop() }
    @objc static func goBack() { shared?.store.goBack() }
    @objc static func goForward() { shared?.store.goForward() }
    @objc static func goHome() { shared?.store.goHome() }
    @objc static func toggleSidebar() { shared?.store.toggleSidebar() }
    @objc static func toggleAIPanel() { shared?.store.toggleAIPanel() }
    @objc static func openSettings() { shared?.store.settingsVisible = true }
    @objc static func focusOmnibox() { shared?.store.presentLauncherForCurrentTab() }
    @objc static func zoomIn() { shared?.store.zoomIn() }
    @objc static func zoomOut() { shared?.store.zoomOut() }
    @objc static func resetZoom() { shared?.store.resetZoom() }
    @objc static func toggleFindBar() { shared?.store.toggleFindBar() }
    @objc static func findNext() { shared?.store.findNext(forward: true) }
    @objc static func findPrevious() { shared?.store.findNext(forward: false) }
    @objc static func toggleDevTools() { shared?.store.toggleDevTools() }
    @objc static func printPage() { shared?.store.printPage() }
    @objc static func selectNextTab() { shared?.store.selectNextTab() }
    @objc static func selectPreviousTab() { shared?.store.selectPreviousTab() }

}
