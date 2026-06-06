import SwiftUI
import AppKit

/// Bridge object the ObjC++ AppDelegate calls to build and own the SwiftUI
/// chrome. Holds the single shared BrowserStore for the window.
@objc(MoriRoot)
final class MoriRoot: NSObject {
    /// Retained for the app lifetime so the store/tabs aren't deallocated.
    private static var shared: MoriRoot?

    let store = BrowserStore()

    @objc static func makeRootViewController() -> NSViewController {
        let root = MoriRoot()
        shared = root

        let hosting = NSHostingController(rootView: RootView(store: root.store))
        hosting.view.frame = NSRect(x: 0, y: 0, width: 1280, height: 820)
        return hosting
    }

    @objc static func prepareForTermination() {
        shared?.store.prepareForTermination()
    }

    @objc static func handleShortcutEvent(_ event: NSEvent) -> Bool {
        guard let store = shared?.store else { return false }
        return MoriCommands.handle(event, store: store)
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

    // Menu-driven actions (called from the AppKit menu bar).
    // ⌘T / File ▸ New Tab opens the launcher (command palette) rather than
    // silently spawning a blank tab.
    @objc static func newTab() { shared?.store.presentLauncher() }
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
    @objc static func focusOmnibox() {
        NotificationCenter.default.post(name: .moriFocusOmnibox, object: nil)
    }
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

    @objc static func handleExtensionTabs(_ method: String,
                                          args: NSDictionary) -> NSDictionary {
        guard let store = shared?.store else {
            return ["error": "Browser store is not ready."]
        }
        return store.handleExtensionTabs(method: method, args: args)
    }

    @objc static func handleExtensionWindows(_ method: String,
                                             args: NSDictionary) -> NSDictionary {
        guard let store = shared?.store else {
            return ["error": "Browser store is not ready."]
        }
        return store.handleExtensionWindows(method: method, args: args)
    }

    @objc static func handleExtensionDownloads(_ method: String,
                                                args: NSDictionary) -> NSDictionary {
        guard let store = shared?.store else {
            return ["error": "Browser store is not ready."]
        }
        return store.handleExtensionDownloads(method: method, args: args)
    }

    @objc static func handleExtensionSessions(_ method: String,
                                              args: NSDictionary) -> NSDictionary {
        guard let store = shared?.store else {
            return ["error": "Browser store is not ready."]
        }
        return store.handleExtensionSessions(method: method, args: args)
    }

    @objc static func handleExtensionScripting(_ method: String,
                                               args: NSDictionary) -> NSDictionary {
        guard let store = shared?.store else {
            return ["error": "Browser store is not ready."]
        }
        return store.handleExtensionScripting(method: method, args: args)
    }

    @objc static func handleExtensionAction(_ method: String,
                                            args: NSDictionary) -> NSDictionary {
        guard let extensionID = args["extensionId"] as? String, !extensionID.isEmpty else {
            return ["error": "Missing extension id."]
        }
        return ExtensionStore.shared.handleAction(method: method,
                                                 args: args,
                                                 extensionID: extensionID)
    }

    @objc static func handleExtensionManagement(_ method: String,
                                                args: NSDictionary) -> NSDictionary {
        guard let extensionID = args["extensionId"] as? String, !extensionID.isEmpty else {
            return ["error": "Missing extension id."]
        }
        return ExtensionStore.shared.handleManagement(method: method,
                                                     args: args,
                                                     extensionID: extensionID)
    }

    @objc static func handleExtensionBookmarks(_ method: String,
                                               args: NSDictionary) -> NSDictionary {
        BookmarkStore.shared.handleExtensionBookmarks(method: method, args: args)
    }

    @objc static func handleExtensionHistory(_ method: String,
                                             args: NSDictionary) -> NSDictionary {
        HistoryStore.shared.handleExtensionHistory(method: method, args: args)
    }

    @objc static func handleExtensionBrowsingData(_ method: String,
                                                  args: NSDictionary) -> NSDictionary {
        guard let store = shared?.store else {
            return ["error": "Browser store is not ready."]
        }
        return store.handleExtensionBrowsingData(method: method, args: args)
    }

    @objc static func handleExtensionRuntime(_ method: String,
                                             args: NSDictionary) -> NSDictionary {
        guard let store = shared?.store else {
            return ["error": "Browser store is not ready."]
        }
        return store.handleExtensionRuntime(method: method, args: args)
    }
}

extension Notification.Name {
    static let moriFocusOmnibox = Notification.Name("MoriFocusOmnibox")
    static let moriOpenExtensionPopup = Notification.Name("MoriOpenExtensionPopup")
    static let moriOpenExtensionUninstallURL = Notification.Name("MoriOpenExtensionUninstallURL")
}
