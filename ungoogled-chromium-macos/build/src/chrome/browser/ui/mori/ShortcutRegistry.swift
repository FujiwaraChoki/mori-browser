import AppKit

struct MoriShortcutTrigger {
    static let shortcutModifierMask: NSEvent.ModifierFlags = [
        .command, .shift, .option, .control
    ]

    let modifiers: NSEvent.ModifierFlags
    let key: String
    let keyAliases: Set<String>
    let keyCode: UInt16
    let isRepeat: Bool

    init(event: NSEvent) {
        keyCode = event.keyCode
        modifiers = event.modifierFlags.intersection(Self.shortcutModifierMask)
        let keys = Self.normalizedAppKitKeys(for: event)
        key = keys.primary
        keyAliases = keys.aliases
        isRepeat = event.isARepeat
    }

    init(keyCode: UInt16,
         charactersIgnoringModifiers: String?,
         modifierMask: UInt,
         isRepeat: Bool) {
        self.keyCode = keyCode
        modifiers = NSEvent.ModifierFlags(rawValue: modifierMask)
            .intersection(Self.shortcutModifierMask)
        let keys = Self.normalizedCEFKeys(
            keyCode: keyCode,
            charactersIgnoringModifiers: charactersIgnoringModifiers)
        key = keys.primary
        keyAliases = keys.aliases
        self.isRepeat = isRepeat
    }

    func matchesKey(_ key: String) -> Bool {
        keyAliases.contains(key)
    }

    private static func normalizedAppKitKeys(for event: NSEvent)
        -> (primary: String, aliases: Set<String>) {
        if let special = normalizedSpecialAppKitKey(keyCode: event.keyCode) {
            return (special, [special])
        }

        let logical = normalizedPrintableKey(event.charactersIgnoringModifiers ?? "")
        let physical = normalizedPhysicalAppKitKey(keyCode: event.keyCode)
        var aliases = Set<String>()
        if !logical.isEmpty { aliases.insert(logical) }
        if let physical, !physical.isEmpty { aliases.insert(physical) }
        let primary = !logical.isEmpty ? logical : (physical ?? "")
        if !primary.isEmpty { aliases.insert(primary) }
        return (primary, aliases)
    }

    private static func normalizedSpecialAppKitKey(keyCode: UInt16) -> String? {
        switch keyCode {
        case 48: return "tab"
        case 53: return "escape"
        case 116: return "pageup"
        case 121: return "pagedown"
        case 123: return "left"
        case 124: return "right"
        case 125: return "down"
        case 126: return "up"
        default: return nil
        }
    }

    private static func normalizedPhysicalAppKitKey(keyCode: UInt16) -> String? {
        switch keyCode {
        case 0: return "a"
        case 1: return "s"
        case 2: return "d"
        case 3: return "f"
        case 4: return "h"
        case 5: return "g"
        case 6: return "z"
        case 7: return "x"
        case 8: return "c"
        case 9: return "v"
        case 11: return "b"
        case 12: return "q"
        case 13: return "w"
        case 14: return "e"
        case 15: return "r"
        case 16: return "y"
        case 17: return "t"
        case 18: return "1"
        case 19: return "2"
        case 20: return "3"
        case 21: return "4"
        case 22: return "6"
        case 23: return "5"
        case 24: return "="
        case 25: return "9"
        case 26: return "7"
        case 27: return "-"
        case 28: return "8"
        case 29: return "0"
        case 30: return "]"
        case 31: return "o"
        case 32: return "u"
        case 33: return "["
        case 34: return "i"
        case 35: return "p"
        case 37: return "l"
        case 38: return "j"
        case 40: return "k"
        case 43: return ","
        case 45: return "n"
        case 46: return "m"
        case 47: return "."
        default: return nil
        }
    }

    private static func normalizedCEFKeys(keyCode: UInt16,
                                          charactersIgnoringModifiers: String?)
        -> (primary: String, aliases: Set<String>) {
        let logical = normalizedPrintableKey(charactersIgnoringModifiers ?? "")
        let physical: String?
        switch keyCode {
        case 9: physical = "tab"
        case 27: physical = "escape"
        case 33: physical = "pageup"
        case 34: physical = "pagedown"
        case 37: physical = "left"
        case 38: physical = "up"
        case 39: physical = "right"
        case 40: physical = "down"
        case 48...57:
            physical = String(UnicodeScalar(Int(keyCode))!)
        case 65...90:
            physical = String(UnicodeScalar(Int(keyCode) + 32)!)
        case 96...105:
            physical = String(Int(keyCode - 96))
        case 107: physical = "+"
        case 109, 189: physical = "-"
        case 187: physical = "="
        case 188: physical = ","
        case 190: physical = "."
        case 219: physical = "["
        case 221: physical = "]"
        default: physical = nil
        }

        var aliases = Set<String>()
        if !logical.isEmpty { aliases.insert(logical) }
        if let physical, !physical.isEmpty { aliases.insert(physical) }
        let primary = !logical.isEmpty ? logical : (physical ?? "")
        if !primary.isEmpty { aliases.insert(primary) }
        return (primary, aliases)
    }

    static func normalizedPrintableKey(_ raw: String) -> String {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        switch trimmed.lowercased() {
        case "{": return "["
        case "}": return "]"
        case "\u{F700}": return "up"
        case "\u{F701}": return "down"
        case "\u{F702}": return "left"
        case "\u{F703}": return "right"
        case "space": return " "
        case "comma": return ","
        case "period": return "."
        case "esc", "escape": return "escape"
        case "tab": return "tab"
        case "pageup", "page up": return "pageup"
        case "pagedown", "page down": return "pagedown"
        case "left", "arrowleft": return "left"
        case "right", "arrowright": return "right"
        case "up", "arrowup": return "up"
        case "down", "arrowdown": return "down"
        default: return trimmed.lowercased()
        }
    }
}

private struct MoriShortcut {
    let id: String
    let modifiers: NSEvent.ModifierFlags
    let keys: Set<String>
    let acceptsRepeats: Bool
    let reservesChromiumShortcut: Bool
    let isEnabled: (BrowserStore, MoriShortcutTrigger) -> Bool
    let perform: (BrowserStore) -> Void

    init(_ id: String,
         modifiers: NSEvent.ModifierFlags,
         key: String,
         acceptsRepeats: Bool = false,
         reservesChromiumShortcut: Bool = false,
         isEnabled: @escaping (BrowserStore, MoriShortcutTrigger) -> Bool = { _, _ in true },
         perform: @escaping (BrowserStore) -> Void) {
        self.init(id,
                  modifiers: modifiers,
                  keys: [key],
                  acceptsRepeats: acceptsRepeats,
                  reservesChromiumShortcut: reservesChromiumShortcut,
                  isEnabled: isEnabled,
                  perform: perform)
    }

    init(_ id: String,
         modifiers: NSEvent.ModifierFlags,
         keys: Set<String>,
         acceptsRepeats: Bool = false,
         reservesChromiumShortcut: Bool = false,
         isEnabled: @escaping (BrowserStore, MoriShortcutTrigger) -> Bool = { _, _ in true },
         perform: @escaping (BrowserStore) -> Void) {
        self.id = id
        self.modifiers = modifiers
        self.keys = keys
        self.acceptsRepeats = acceptsRepeats
        self.reservesChromiumShortcut = reservesChromiumShortcut
        self.isEnabled = isEnabled
        self.perform = perform
    }

    func matches(_ trigger: MoriShortcutTrigger, store: BrowserStore) -> Bool {
        modifiers == trigger.modifiers &&
            !keys.isDisjoint(with: trigger.keyAliases) &&
            isEnabled(store, trigger)
    }
}

/// Single registry for browser keyboard shortcuts.
///
/// New shortcuts should be added to `shortcuts` below. Both native AppKit
/// key events and CEF key events normalize into `MoriShortcutTrigger`, so a
/// shortcut registered here works from chrome focus and web-content focus.
enum MoriCommands {
    private struct ShortcutEventIdentity: Equatable {
        let timestamp: TimeInterval
        let keyCode: UInt16
        let key: String
        let modifiersRawValue: UInt

        init(event: NSEvent, trigger: MoriShortcutTrigger) {
            timestamp = event.timestamp
            keyCode = event.keyCode
            key = trigger.key
            modifiersRawValue = trigger.modifiers.rawValue
        }
    }

    private static var lastHandledEvent: ShortcutEventIdentity?

    static func handle(_ event: NSEvent, store: BrowserStore) -> Bool {
        guard event.type == .keyDown else { return false }
        let trigger = MoriShortcutTrigger(event: event)
        let eventIdentity = ShortcutEventIdentity(event: event, trigger: trigger)
        return handle(trigger, eventIdentity: eventIdentity, store: store)
    }

    static func release(_ event: NSEvent) {
    }

    static func release(keyCode: UInt16,
                        charactersIgnoringModifiers: String?,
                        modifierMask: UInt) {
    }

    static func handle(keyCode: UInt16,
                       charactersIgnoringModifiers: String?,
                       modifierMask: UInt,
                       isRepeat: Bool,
                       store: BrowserStore) -> Bool {
        let trigger = MoriShortcutTrigger(
            keyCode: keyCode,
            charactersIgnoringModifiers: charactersIgnoringModifiers,
            modifierMask: modifierMask,
            isRepeat: isRepeat)
        return handle(trigger, eventIdentity: nil, store: store)
    }

    static func reservesChromiumShortcut(keyEquivalent: String,
                                         modifierMask: UInt) -> Bool {
        let trigger = MoriShortcutTrigger(
            keyCode: 0,
            charactersIgnoringModifiers: keyEquivalent,
            modifierMask: modifierMask,
            isRepeat: false)
        return shortcuts.contains {
            $0.reservesChromiumShortcut &&
                $0.modifiers == trigger.modifiers &&
                !$0.keys.isDisjoint(with: trigger.keyAliases)
        }
    }

    private static func handle(_ trigger: MoriShortcutTrigger,
                               eventIdentity: ShortcutEventIdentity?,
                               store: BrowserStore) -> Bool {
        if let eventIdentity, eventIdentity == lastHandledEvent {
            return true
        }

        if let shortcut = shortcuts.first(where: { $0.matches(trigger, store: store) }) {
            if trigger.isRepeat && !shortcut.acceptsRepeats {
                remember(eventIdentity)
                return true
            }
            // Perform synchronously so the store mutation is done by the time
            // the caller (MoriRoot.handleShortcutEvent) forces the chrome to
            // repaint — see flushChrome(). AppKit and Chromium can both report
            // the same physical key event, so exact event identity collapses
            // duplicate delivery without blocking a later fresh press.
            shortcut.perform(store)
            remember(eventIdentity)
            return true
        }

        if isTextEditingShortcut(trigger) {
            return false
        }

        if let command = ExtensionStore.shared.command(matching: trigger) {
            ExtensionStore.shared.activate(command)
            return true
        }

        return false
    }

    private static func remember(_ eventIdentity: ShortcutEventIdentity?) {
        if let eventIdentity {
            lastHandledEvent = eventIdentity
        }
    }

    private static let shortcuts: [MoriShortcut] = {
        var result: [MoriShortcut] = [
            MoriShortcut("finishZapMode",
                         modifiers: [],
                         key: "escape",
                         isEnabled: { store, _ in store.zapModeActive }) {
                $0.finishZapMode()
            },
            MoriShortcut("dismissBoostEditor",
                         modifiers: [],
                         key: "escape",
                         isEnabled: { store, _ in store.boostEditorVisible }) {
                $0.dismissBoostEditor()
            },
            MoriShortcut("dismissPeek",
                         modifiers: [],
                         key: "escape",
                         isEnabled: { store, _ in store.peekTab != nil }) {
                $0.closePeek()
            },
            MoriShortcut("dismissWebContextMenu",
                         modifiers: [],
                         key: "escape",
                         isEnabled: { store, _ in store.contextMenu != nil }) {
                $0.dismissWebContextMenu()
            },
            MoriShortcut("cancelCapture",
                         modifiers: [],
                         key: "escape",
                         isEnabled: { store, _ in store.captureMode }) {
                $0.cancelRegionCapture()
            },
            MoriShortcut("dismissLauncher",
                         modifiers: [],
                         key: "escape",
                         isEnabled: { store, _ in store.launcherVisible }) {
                $0.dismissLauncher()
            },
            MoriShortcut("dismissFindBar",
                         modifiers: [],
                         key: "escape",
                         isEnabled: { store, _ in store.findBarVisible }) {
                $0.hideFindBar()
            },
            MoriShortcut("dismissSettings",
                         modifiers: [],
                         key: "escape",
                         isEnabled: { store, _ in store.settingsVisible }) {
                $0.settingsVisible = false
            },
            MoriShortcut("dismissContextCreation",
                         modifiers: [],
                         key: "escape",
                         isEnabled: { store, _ in store.contextCreationVisible }) {
                $0.contextCreationVisible = false
            },
            MoriShortcut("dismissErrorOverlay",
                         modifiers: [],
                         key: "escape",
                         isEnabled: { store, _ in (store.selectedTab ?? store.tabs.first)?.didFail == true }) {
                if let tab = $0.selectedTab ?? $0.tabs.first {
                    tab.didFail = false
                    tab.failError = ""
                }
            },
            MoriShortcut("newSplit", modifiers: [.control, .shift], keys: ["=", "+"]) {
                $0.newSplit()
            },
            MoriShortcut("toggleDevTools", modifiers: [.command, .option], key: "i") {
                $0.toggleDevTools()
            },
            MoriShortcut("nextTabCommandOption",
                         modifiers: [.command, .option],
                         key: "right",
                         acceptsRepeats: true) {
                $0.selectNextTab()
            },
            MoriShortcut("previousTabCommandOption",
                         modifiers: [.command, .option],
                         key: "left",
                         acceptsRepeats: true) {
                $0.selectPreviousTab()
            },
            MoriShortcut("toggleAIOptionA",
                         modifiers: .option,
                         key: "a",
                         isEnabled: { store, _ in store.settings.aiIntegrationEnabled }) {
                $0.toggleAIPanel()
            },
            MoriShortcut("nextTabCommandShiftBracket",
                         modifiers: [.command, .shift],
                         key: "]",
                         acceptsRepeats: true) {
                $0.selectNextTab()
            },
            MoriShortcut("previousTabCommandShiftBracket",
                         modifiers: [.command, .shift],
                         key: "[",
                         acceptsRepeats: true) {
                $0.selectPreviousTab()
            },
            MoriShortcut("reopenClosedTab", modifiers: [.command, .shift], key: "t") {
                $0.reopenClosedTab()
            },
            MoriShortcut("peek", modifiers: [.command, .shift], key: "o") {
                $0.peekFromClipboardOrCurrent()
            },
            MoriShortcut("boostSite", modifiers: [.command, .shift], key: "b") {
                $0.presentBoostEditor()
            },
            MoriShortcut("sleepBackgroundTabs", modifiers: [.command, .control], key: "s") {
                $0.sleepBackgroundTabs()
            },
            MoriShortcut("copyCurrentURL", modifiers: [.command, .shift], key: "c") {
                $0.copyCurrentTabURL()
            },
            MoriShortcut("duplicateTab", modifiers: [.command, .shift], key: "d") {
                if let id = $0.selectedTabID { _ = $0.duplicateTab(id) }
            },
            MoriShortcut("togglePinTab", modifiers: [.command, .shift], key: "p") {
                if let id = $0.selectedTabID { $0.togglePin(id) }
            },
            MoriShortcut("closeSplit", modifiers: [.command, .shift], key: "s") {
                $0.closeSplit()
            },
            MoriShortcut("forceReload", modifiers: [.command, .shift], key: "r") {
                $0.reloadIgnoringCache()
            },
            MoriShortcut("findPrevious", modifiers: [.command, .shift], key: "g") {
                $0.findNext(forward: false)
            },
            MoriShortcut("home", modifiers: [.command, .shift], key: "h") {
                $0.goHome()
            },
            MoriShortcut("zoomInShift",
                         modifiers: [.command, .shift],
                         keys: ["=", "+"],
                         acceptsRepeats: true) {
                $0.zoomIn()
            },
            MoriShortcut("nextTabControlTab",
                         modifiers: .control,
                         key: "tab",
                         acceptsRepeats: true) {
                $0.selectNextTab()
            },
            MoriShortcut("previousTabControlTab",
                         modifiers: [.control, .shift],
                         key: "tab",
                         acceptsRepeats: true) {
                $0.selectPreviousTab()
            },
            MoriShortcut("nextTabControlPageDown",
                         modifiers: .control,
                         key: "pagedown",
                         acceptsRepeats: true) {
                $0.selectNextTab()
            },
            MoriShortcut("previousTabControlPageUp",
                         modifiers: .control,
                         key: "pageup",
                         acceptsRepeats: true) {
                $0.selectPreviousTab()
            },
            MoriShortcut("toggleSidebar",
                         modifiers: .command,
                         key: "s",
                         reservesChromiumShortcut: true) {
                $0.toggleSidebar()
            },
            MoriShortcut("toggleSidebarControl", modifiers: .control, key: "s") {
                $0.toggleSidebar()
            },
            MoriShortcut("toggleOmnibox",
                         modifiers: .command,
                         key: "t",
                         reservesChromiumShortcut: true) {
                $0.toggleLauncher()
            },
            MoriShortcut("closeTab", modifiers: .command, key: "w") {
                if let id = $0.selectedTabID { $0.closeTab(id) }
            },
            MoriShortcut("focusOmnibox",
                         modifiers: .command,
                         key: "l",
                         reservesChromiumShortcut: true) {
                $0.presentLauncherForCurrentTab()
            },
            MoriShortcut("reload", modifiers: .command, key: "r") {
                $0.reload()
            },
            MoriShortcut("print", modifiers: .command, key: "p") {
                $0.printPage()
            },
            MoriShortcut("find", modifiers: .command, key: "f") {
                $0.toggleFindBar()
            },
            MoriShortcut("findNext", modifiers: .command, key: "g") {
                $0.findNext(forward: true)
            },
            MoriShortcut("toggleAI",
                         modifiers: .command,
                         key: "k",
                         isEnabled: { store, _ in store.settings.aiIntegrationEnabled }) {
                $0.toggleAIPanel()
            },
            MoriShortcut("stop", modifiers: .command, key: ".") {
                $0.stop()
            },
            MoriShortcut("zoomIn", modifiers: .command, key: "=", acceptsRepeats: true) {
                $0.zoomIn()
            },
            MoriShortcut("zoomOut", modifiers: .command, key: "-", acceptsRepeats: true) {
                $0.zoomOut()
            },
            MoriShortcut("resetZoom", modifiers: .command, key: "0") {
                $0.resetZoom()
            },
            MoriShortcut("back", modifiers: .command, key: "[") {
                $0.goBack()
            },
            MoriShortcut("forward", modifiers: .command, key: "]") {
                $0.goForward()
            },
            MoriShortcut("settings", modifiers: .command, key: ",") {
                $0.settingsVisible = true
            },
            MoriShortcut("hide", modifiers: .command, key: "h") { _ in
                NSApp.hide(nil)
            },
            MoriShortcut("minimize", modifiers: .command, key: "m") { _ in
                (NSApp.keyWindow ?? NSApp.mainWindow)?.performMiniaturize(nil)
            },
            MoriShortcut("quit", modifiers: .command, key: "q") { _ in
                NSApp.terminate(nil)
            }
        ]

        for ordinal in 1...9 {
            result.append(MoriShortcut("selectTab\(ordinal)",
                                       modifiers: .command,
                                       key: String(ordinal),
                                       acceptsRepeats: true) {
                $0.selectTab(atOrdinal: ordinal)
            })
        }

        for ordinal in 1...9 {
            result.append(MoriShortcut("switchContext\(ordinal)",
                                       modifiers: .control,
                                       key: String(ordinal)) {
                $0.switchContext(atOrdinal: ordinal)
            })
        }

        return result
    }()

    private static func isTextEditingShortcut(_ trigger: MoriShortcutTrigger) -> Bool {
        if trigger.modifiers == .command {
            return ["a", "c", "v", "x", "z"].contains {
                trigger.matchesKey($0)
            }
        }
        if trigger.modifiers == [.command, .shift], trigger.matchesKey("z") {
            return true
        }
        return false
    }
}
