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
        guard let view = chromeView else { return }
        view.needsLayout = true
        view.layoutSubtreeIfNeeded()
        view.displayIfNeeded()
        NSLog("MORI-KEY flushChrome did layout")
    }

    @objc static func prepareForTermination() {
        shared?.store.prepareForTermination()
    }

    @objc static func handleShortcutEvent(_ event: NSEvent) -> Bool {
        guard let store = shared?.store else { return false }
        let handled = MoriCommands.handle(event, store: store)
        if handled { flushChrome() }
        return handled
    }

    @objc static func releaseShortcutEvent(_ event: NSEvent) {
        MoriCommands.release(event)
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
        guard let store = shared?.store else { return false }
        return MoriCommands.handle(keyCode: keyCode,
                                   charactersIgnoringModifiers: charactersIgnoringModifiers,
                                   modifierMask: modifierMask,
                                   isRepeat: isRepeat,
                                   store: store)
    }

    @objc(releaseShortcutWithKeyCode:charactersIgnoringModifiers:modifierMask:)
    static func releaseShortcut(keyCode: UInt16,
                                charactersIgnoringModifiers: String?,
                                modifierMask: UInt) {
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
