import SwiftUI

struct ArcBar: View {
    var store: BrowserStore
    var embedded: Bool
    var showsTabs: Bool
    @Environment(\.moriPalette) private var palette

    var body: some View {
        HStack(spacing: 10) {
            HStack(spacing: 0) {
                Button(action: store.goBack) {
                    Image(systemName: "chevron.left")
                        .font(.system(size: 17, weight: .semibold))
                        .frame(width: 44, height: 44)
                        .opacity(store.activeSession?.canGoBack == true ? 1 : 0.28)
                }
                .buttonStyle(.plain)
                .disabled(store.activeSession?.canGoBack != true)
                .accessibilityLabel("Back")

                Button(action: store.beginNavigation) {
                    HStack(spacing: 5) {
                        if store.currentURL?.scheme?.lowercased() == "https" {
                            Image(systemName: "lock.fill")
                                .font(.system(size: 10, weight: .semibold))
                                .foregroundStyle(palette.mutedForeground)
                                .accessibilityLabel("Secure")
                        }
                        Text(title)
                            .font(.system(size: 16, weight: .semibold))
                            .foregroundStyle(store.addressIsPlaceholder ? palette.mutedForeground : palette.foreground)
                            .lineLimit(1)
                    }
                    .frame(maxWidth: .infinity)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Address, \(title)")
                .accessibilityIdentifier("addressPrompt")
                .contextMenu { pageActions }

                if store.activeSession?.isLoading == true {
                    Button(action: store.stop) {
                        Image(systemName: "xmark")
                            .font(.system(size: 12, weight: .bold))
                            .frame(width: 28, height: 44)
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Stop")
                }

                PageMenu(store: store, embedded: embedded, showsTabs: showsTabs)
            }
            .foregroundStyle(palette.foreground)
            .padding(.leading, 4)
            .padding(.trailing, 4)
            .frame(height: 52)
            .background { barFill }

            Button(action: store.newTab) {
                Image(systemName: "plus")
                    .font(.system(size: 20, weight: .semibold))
                    .foregroundStyle(palette.foreground)
                    .frame(width: 52, height: 52)
                    .background { barFill }
            }
            .buttonStyle(.plain)
            .accessibilityLabel("New Tab")
            .accessibilityIdentifier("newTabButton")
        }
    }

    private var barFill: some View {
        ZStack {
            Capsule().fill(.ultraThinMaterial)
            Capsule().strokeBorder(Color.primary.opacity(0.08), lineWidth: 0.5)
        }
        .shadow(color: .black.opacity(0.14), radius: 18, y: 8)
    }

    @ViewBuilder
    private var pageActions: some View {
        if showsTabs {
            Button("Tabs") { store.sidebarPresented = true }
        }
        Button("Forward", action: store.goForward)
            .disabled(store.activeSession?.canGoForward != true)
        Button(store.activeSession?.isLoading == true ? "Stop" : "Reload") {
            if store.activeSession?.isLoading == true { store.stop() } else { store.reload() }
        }
        .disabled(store.addressIsPlaceholder && store.activeSession?.isLoading != true)
        Button("Find in Page", action: store.openFind)
            .disabled(!store.hasPage)
        Button("Reader", action: store.openReader)
            .disabled(!store.hasPage)
        Divider()
        Button(store.currentIsBookmarked ? "Remove Bookmark" : "Add Bookmark", action: store.toggleBookmark)
            .disabled(!store.hasPage)
        Button("Share", action: store.shareCurrent)
            .disabled(!store.hasPage)
        Button("Copy Link", action: store.copyLink)
            .disabled(!store.hasPage)
    }

    private var title: String {
        guard let host = store.currentURL?.host else { return "Search" }
        if host.hasPrefix("www.") { return String(host.dropFirst(4)) }
        return host
    }
}

struct PageMenu: View {
    var store: BrowserStore
    var embedded: Bool
    var showsTabs: Bool
    @Environment(\.moriPalette) private var palette

    var body: some View {
        Menu {
            if showsTabs {
                Button("Tabs") { store.sidebarPresented = true }
            }
            Button("Forward", action: store.goForward)
                .disabled(store.activeSession?.canGoForward != true)
            Button(store.activeSession?.isLoading == true ? "Stop" : "Reload") {
                if store.activeSession?.isLoading == true { store.stop() } else { store.reload() }
            }
            .disabled(store.addressIsPlaceholder && store.activeSession?.isLoading != true)
            Button("Find in Page", action: store.openFind)
                .disabled(!store.hasPage)
            Button("Reader", action: store.openReader)
                .disabled(!store.hasPage)
            Divider()
            Button(store.currentIsBookmarked ? "Remove Bookmark" : "Add Bookmark", action: store.toggleBookmark)
                .disabled(!store.hasPage)
            Button("Share", action: store.shareCurrent)
                .disabled(!store.hasPage)
            Button("Copy Link", action: store.copyLink)
                .disabled(!store.hasPage)
            Menu("Zoom") {
                Button("Smaller") { store.adjustZoom(by: -0.1) }
                Button("Default", action: store.resetZoom)
                Button("Larger") { store.adjustZoom(by: 0.1) }
            }
            .disabled(!store.hasPage)
            Divider()
            Button("New Tab", action: store.newTab)
            Button("Library") { store.open(.library, embedded: embedded) }
            Button("Settings") { store.open(.settings, embedded: embedded) }
        } label: {
            Image(systemName: "ellipsis")
                .font(.system(size: 17, weight: .semibold))
                .frame(width: 36, height: 44)
                .foregroundStyle(palette.foreground)
                .contentShape(Rectangle())
        }
        .accessibilityLabel("Page menu")
    }
}

struct FindBar: View {
    @Bindable var store: BrowserStore
    @Environment(\.moriPalette) private var palette
    @FocusState private var focused: Bool

    var body: some View {
        HStack(spacing: 8) {
            TextField("Find", text: $store.findText)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .focused($focused)
                .submitLabel(.search)
                .onSubmit { store.find(forward: true) }
            Button { store.find(forward: false) } label: {
                Image(systemName: "chevron.up").frame(width: 32, height: 32)
            }
            .accessibilityLabel("Previous match")
            Button { store.find(forward: true) } label: {
                Image(systemName: "chevron.down").frame(width: 32, height: 32)
            }
            .accessibilityLabel("Next match")
            Button(action: store.closeFind) {
                Image(systemName: "xmark").frame(width: 32, height: 32)
            }
            .accessibilityLabel("Close find")
        }
        .buttonStyle(.plain)
        .foregroundStyle(palette.foreground)
        .padding(.horizontal, 14)
        .padding(.vertical, 6)
        .background(.ultraThinMaterial)
        .onAppear { focused = true }
    }
}

struct PageDialogModifier: ViewModifier {
    var store: BrowserStore
    @State private var promptText = ""

    func body(content: Content) -> some View {
        content
            .alert(
                "This page says",
                isPresented: Binding(
                    get: { store.activeSession?.dialog != nil },
                    set: { if !$0 { store.activeSession?.dismissDialog(.cancel) } }
                ),
                presenting: store.activeSession?.dialog
            ) { dialog in
                switch dialog {
                case .alert:
                    Button("OK") { store.activeSession?.dismissDialog(.ok(nil)) }
                case .confirm:
                    Button("Cancel", role: .cancel) { store.activeSession?.dismissDialog(.cancel) }
                    Button("OK") { store.activeSession?.dismissDialog(.ok(nil)) }
                case .prompt:
                    TextField("Reply", text: $promptText)
                    Button("Cancel", role: .cancel) { store.activeSession?.dismissDialog(.cancel) }
                    Button("OK") { store.activeSession?.dismissDialog(.ok(promptText)) }
                }
            } message: { dialog in
                Text(dialog.message)
            }
            .onChange(of: store.activeSession?.dialog) { _, dialog in
                if case .prompt(_, let value) = dialog {
                    promptText = value
                }
            }
    }
}
