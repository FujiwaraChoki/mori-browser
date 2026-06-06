import AppKit

struct MoriShortcutTrigger {
    private static let shortcutModifierMask: NSEvent.ModifierFlags = [
        .command, .shift, .option, .control
    ]

    let modifiers: NSEvent.ModifierFlags
    let key: String
    let keyCode: UInt16
    let isRepeat: Bool

    init(event: NSEvent) {
        keyCode = event.keyCode
        modifiers = event.modifierFlags.intersection(Self.shortcutModifierMask)
        key = Self.normalizedAppKitKey(for: event)
        isRepeat = event.isARepeat
    }

    init(keyCode: UInt16,
         charactersIgnoringModifiers: String?,
         modifierMask: UInt,
         isRepeat: Bool) {
        self.keyCode = keyCode
        modifiers = NSEvent.ModifierFlags(rawValue: modifierMask)
            .intersection(Self.shortcutModifierMask)
        key = Self.normalizedCEFKey(keyCode: keyCode,
                                    charactersIgnoringModifiers: charactersIgnoringModifiers)
        self.isRepeat = isRepeat
    }

    private static func normalizedAppKitKey(for event: NSEvent) -> String {
        switch event.keyCode {
        case 48: return "tab"
        case 53: return "escape"
        case 116: return "pageup"
        case 121: return "pagedown"
        case 123: return "left"
        case 124: return "right"
        case 125: return "down"
        case 126: return "up"
        default:
            break
        }

        guard let chars = event.charactersIgnoringModifiers, !chars.isEmpty else {
            return ""
        }

        return normalizedPrintableKey(chars)
    }

    private static func normalizedCEFKey(keyCode: UInt16,
                                         charactersIgnoringModifiers: String?) -> String {
        switch keyCode {
        case 9: return "tab"
        case 27: return "escape"
        case 33: return "pageup"
        case 34: return "pagedown"
        case 37: return "left"
        case 38: return "up"
        case 39: return "right"
        case 40: return "down"
        case 48...57:
            return String(UnicodeScalar(Int(keyCode))!)
        case 65...90:
            return String(UnicodeScalar(Int(keyCode) + 32)!)
        case 96...105:
            return String(Int(keyCode - 96))
        case 107: return "+"
        case 109, 189: return "-"
        case 187: return "="
        case 188: return ","
        case 190: return "."
        case 219: return "["
        case 221: return "]"
        default:
            return normalizedPrintableKey(charactersIgnoringModifiers ?? "")
        }
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
    let isEnabled: (BrowserStore, MoriShortcutTrigger) -> Bool
    let perform: (BrowserStore) -> Void

    init(_ id: String,
         modifiers: NSEvent.ModifierFlags,
         key: String,
         acceptsRepeats: Bool = false,
         isEnabled: @escaping (BrowserStore, MoriShortcutTrigger) -> Bool = { _, _ in true },
         perform: @escaping (BrowserStore) -> Void) {
        self.init(id,
                  modifiers: modifiers,
                  keys: [key],
                  acceptsRepeats: acceptsRepeats,
                  isEnabled: isEnabled,
                  perform: perform)
    }

    init(_ id: String,
         modifiers: NSEvent.ModifierFlags,
         keys: Set<String>,
         acceptsRepeats: Bool = false,
         isEnabled: @escaping (BrowserStore, MoriShortcutTrigger) -> Bool = { _, _ in true },
         perform: @escaping (BrowserStore) -> Void) {
        self.id = id
        self.modifiers = modifiers
        self.keys = keys
        self.acceptsRepeats = acceptsRepeats
        self.isEnabled = isEnabled
        self.perform = perform
    }

    func matches(_ trigger: MoriShortcutTrigger, store: BrowserStore) -> Bool {
        modifiers == trigger.modifiers &&
            keys.contains(trigger.key) &&
            isEnabled(store, trigger)
    }
}

/// Single registry for browser keyboard shortcuts.
///
/// New shortcuts should be added to `shortcuts` below. Both native AppKit
/// key events and CEF key events normalize into `MoriShortcutTrigger`, so a
/// shortcut registered here works from chrome focus and web-content focus.
enum MoriCommands {
    private static var lastHandledShortcut: (id: String, key: String, modifiers: NSEvent.ModifierFlags, time: TimeInterval)?
    private static let duplicateShortcutInterval: TimeInterval = 0.08

    static func handle(_ event: NSEvent, store: BrowserStore) -> Bool {
        guard event.type == .keyDown else { return false }
        return handle(MoriShortcutTrigger(event: event), store: store)
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
        return handle(trigger, store: store)
    }

    private static func handle(_ trigger: MoriShortcutTrigger,
                               store: BrowserStore) -> Bool {
        if let shortcut = shortcuts.first(where: { $0.matches(trigger, store: store) }) {
            if isDuplicate(shortcut, trigger: trigger) {
                return true
            }
            if trigger.isRepeat && !shortcut.acceptsRepeats {
                return true
            }
            shortcut.perform(store)
            remember(shortcut, trigger: trigger)
            return true
        }

        if isTextEditingShortcut(trigger) {
            return false
        }

        if let command = ExtensionStore.shared.command(matching: trigger) {
            store.activateExtensionCommand(command)
            return true
        }

        return false
    }

    private static func isDuplicate(_ shortcut: MoriShortcut,
                                    trigger: MoriShortcutTrigger) -> Bool {
        guard let last = lastHandledShortcut else { return false }
        let now = ProcessInfo.processInfo.systemUptime
        return last.id == shortcut.id &&
            last.key == trigger.key &&
            last.modifiers == trigger.modifiers &&
            now - last.time < duplicateShortcutInterval
    }

    private static func remember(_ shortcut: MoriShortcut,
                                 trigger: MoriShortcutTrigger) {
        lastHandledShortcut = (
            id: shortcut.id,
            key: trigger.key,
            modifiers: trigger.modifiers,
            time: ProcessInfo.processInfo.systemUptime)
    }

    private static let shortcuts: [MoriShortcut] = {
        var result: [MoriShortcut] = [
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
            MoriShortcut("toggleAIOptionA", modifiers: .option, key: "a") {
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
            MoriShortcut("copyCurrentURL", modifiers: [.command, .shift], key: "c") {
                $0.copyCurrentTabURL()
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
            MoriShortcut("toggleSidebarControl", modifiers: .control, key: "s") {
                $0.toggleSidebar()
            },
            MoriShortcut("newTab", modifiers: .command, key: "t") {
                $0.toggleLauncher()
            },
            MoriShortcut("closeTab", modifiers: .command, key: "w") {
                if let id = $0.selectedTabID { $0.closeTab(id) }
            },
            MoriShortcut("focusOmnibox", modifiers: .command, key: "l") { _ in
                NotificationCenter.default.post(name: .moriFocusOmnibox, object: nil)
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
            MoriShortcut("toggleSidebar", modifiers: .command, key: "s") {
                $0.toggleSidebar()
            },
            MoriShortcut("toggleAI", modifiers: .command, key: "k") {
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

        return result
    }()

    private static func isTextEditingShortcut(_ trigger: MoriShortcutTrigger) -> Bool {
        if trigger.modifiers == .command {
            return ["a", "c", "v", "x", "z"].contains(trigger.key)
        }
        if trigger.modifiers == [.command, .shift], trigger.key == "z" {
            return true
        }
        return false
    }
}
