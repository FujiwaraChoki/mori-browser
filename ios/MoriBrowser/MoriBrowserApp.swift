import SwiftUI

@main
struct MoriBrowserApp: App {
    @State private var store = BrowserStore()

    var body: some Scene {
        WindowGroup {
            RootView(store: store)
        }
        .commands {
            CommandGroup(replacing: .newItem) {
                Button("New Tab", action: store.newTab)
                    .keyboardShortcut("t", modifiers: .command)
            }
            CommandGroup(after: .toolbar) {
                Button("Open Command Bar", action: store.beginNavigation)
                    .keyboardShortcut("l", modifiers: .command)
                Button("Find in Page", action: store.openFind)
                    .keyboardShortcut("f", modifiers: .command)
                Button("Reader", action: store.openReader)
                    .keyboardShortcut("r", modifiers: [.command, .shift])
                Button("Bookmark This Page", action: store.toggleBookmark)
                    .keyboardShortcut("d", modifiers: .command)
            }
            CommandGroup(after: .windowArrangement) {
                Button("Close Tab", action: store.closeSelected)
                    .keyboardShortcut("w", modifiers: .command)
                Button("Reopen Closed Tab", action: store.reopenClosed)
                    .keyboardShortcut("t", modifiers: [.command, .shift])
                Button("Back", action: store.goBack)
                    .keyboardShortcut("[", modifiers: .command)
                Button("Forward", action: store.goForward)
                    .keyboardShortcut("]", modifiers: .command)
                Button("Reload", action: store.reload)
                    .keyboardShortcut("r", modifiers: .command)
            }
        }
    }
}
