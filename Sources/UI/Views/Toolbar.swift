import SwiftUI

/// A fixed hairline strip atop the web content: its 4pt height plus the card's
/// 4pt top padding makes the top gap match the 8pt inset on the card's other
/// edges. Acts as the window drag area and shows the page-load progress bar. The
/// revealed titlebar (traffic lights + a slim bar) is handled separately by
/// `TopChromeOverlay`, which floats over the page rather than resizing it.
struct WebTopStrip: View {
    var tab: BrowserTab?

    var body: some View {
        Color.clear
            .frame(maxWidth: .infinity)
            .frame(height: 4)
        .background {
            // Transparent: inherits the unified chrome surface set on the root,
            // so it's the exact same color as the sidebar (no seam).
            WindowDragArea()
                .ignoresSafeArea()
        }
        // The page-load indicator now lives as a muted bar pinned to the bottom
        // edge of the web card (see `RootView.webCard`).
    }
}

/// A slim indeterminate progress bar shown while a page loads. A muted segment
/// sweeps left→right; respects reduced-motion by holding still.
struct LoadingBar: View {
    @Environment(\.palette) private var p
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var phase: CGFloat = 0

    var body: some View {
        GeometryReader { geo in
            let w = geo.size.width
            let segment = max(120, w * 0.28)
            let tint = p.mutedForeground.color
            Capsule()
                .fill(
                    LinearGradient(
                        colors: [tint.opacity(0), tint.opacity(0.7), tint.opacity(0)],
                        startPoint: .leading, endPoint: .trailing
                    )
                )
                .frame(width: segment, height: 2)
                .offset(x: reduceMotion ? (w - segment) / 2 : phase * (w + segment) - segment)
                .onAppear {
                    guard !reduceMotion else { return }
                    withAnimation(.easeInOut(duration: 1.1).repeatForever(autoreverses: false)) {
                        phase = 1
                    }
                }
        }
        .frame(height: 2)
    }
}

/// The address/search field. Shows the page URL when idle; full editable text
/// when focused.
struct Omnibox: View {
    @ObservedObject var store: BrowserStore
    @ObservedObject var tab: BrowserTab
    @ObservedObject private var extensions = ExtensionStore.shared

    @Environment(\.palette) private var p
    @FocusState private var focused: Bool
    @State private var editText: String = ""
    @State private var suggestions: [HistoryEntry] = []
    @State private var highlighted: Int? = nil

    private var showSuggestions: Bool {
        focused && !editText.isEmpty && !suggestions.isEmpty
            && editText != tab.displayURL
    }

    var body: some View {
        HStack(spacing: 7) {
            Icon(name: secureGlyph, size: 13, weight: .regular)
                .foregroundStyle(secureColor)

            ZStack(alignment: .leading) {
                if editText.isEmpty {
                    Text("Search or enter address")
                        .font(Typography.ui(Typography.base))
                        .foregroundStyle(p.mutedForeground.color.opacity(0.7))
                }
                TextField("", text: $editText)
                    .textFieldStyle(.plain)
                    .font(Typography.ui(Typography.base))
                    .foregroundStyle(p.foreground.color)
                    .lineLimit(1)
                    .focused($focused)
                    .onSubmit(submit)
            }

            if let id = ExtensionStore.webStoreExtensionID(from: tab.urlString), !focused {
                AddExtensionButton(installing: extensions.installingIDs.contains(id)) {
                    extensions.beginWebStoreInstall(extensionID: id)
                }
            }

            if !focused {
                ExtensionToolbarItems(store: store)
            }

            if tab.isLoading {
                ProgressView().controlSize(.small).scaleEffect(0.55)
            }
        }
        .padding(.horizontal, 11)
        .frame(height: 30)
        .background {
            if focused {
                // Solid input fill while typing for maximum legibility.
                RoundedRectangle(cornerRadius: Radius.button, style: .continuous)
                    .fill(p.background.color)
            } else {
                // Idle: Apple Liquid Glass capsule.
                Color.clear.liquidGlass(cornerRadius: Radius.button)
            }
        }
        .overlay(
            RoundedRectangle(cornerRadius: Radius.button, style: .continuous)
                .strokeBorder(focused ? p.ring.color.opacity(0.55) : p.border.color.opacity(0.35),
                              lineWidth: focused ? 1.5 : 1)
        )
        // Autocomplete dropdown, floated just below the field.
        .overlay(alignment: .topLeading) {
            if showSuggestions {
                OmniboxSuggestionsList(
                    suggestions: suggestions,
                    highlighted: highlighted,
                    onPick: { commit($0.url) }
                )
                .offset(y: 36)
                .transition(.opacity)
            }
        }
        .animation(Motion.state, value: focused)
        .onAppear { editText = tab.displayURL }
        .onChange(of: focused) { _, now in
            if now {
                DispatchQueue.main.async { selectAll() }
                refreshSuggestions()
            } else {
                // Snap back to the canonical URL when focus leaves.
                editText = tab.displayURL
                suggestions = []
            }
        }
        .onChange(of: editText) { _, _ in
            highlighted = nil
            refreshSuggestions()
        }
        .onChange(of: tab.urlString) { _, _ in
            if !focused { editText = tab.displayURL }
        }
        .onChange(of: tab.id) { _, _ in
            editText = tab.displayURL
        }
        .onReceive(NotificationCenter.default.publisher(for: .moriFocusOmnibox)) { _ in
            focused = true
        }
    }

    private func refreshSuggestions() {
        guard focused, editText != tab.displayURL else { suggestions = []; return }
        suggestions = HistoryStore.shared.suggestions(for: editText, limit: 6)
    }

    private var secureGlyph: String {
        if tab.urlString.hasPrefix("https") { return "lock.fill" }
        if tab.urlString.hasPrefix("http") { return "exclamationmark.triangle" }
        return "magnifyingglass"
    }

    private var secureColor: Color {
        if tab.urlString.hasPrefix("https") { return p.mutedForeground.color }
        if tab.urlString.hasPrefix("http") { return p.statusWarningFg.color }
        return p.mutedForeground.color
    }

    private func submit() {
        // A highlighted suggestion wins; otherwise treat the text as URL/search.
        if let i = highlighted, suggestions.indices.contains(i) {
            commit(suggestions[i].url)
            return
        }
        let text = editText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        store.navigate(text)
        suggestions = []
        focused = false
    }

    /// Navigate straight to a chosen suggestion URL.
    private func commit(_ url: String) {
        store.navigate(url)
        editText = url
        suggestions = []
        focused = false
    }

    private func selectAll() {
        if let editor = NSApp.keyWindow?.firstResponder as? NSTextView {
            editor.selectAll(nil)
        }
    }
}

/// The "Add to Mori" pill shown inside the omnibox on a Chrome Web Store
/// detail page. Tapping it downloads and installs the extension into Mori.
private struct AddExtensionButton: View {
    let installing: Bool
    let action: () -> Void
    @Environment(\.palette) private var p

    var body: some View {
        Button(action: action) {
            HStack(spacing: 4) {
                if installing {
                    ProgressView().controlSize(.small).scaleEffect(0.5)
                        .frame(width: 11, height: 11)
                } else {
                    Icon(name: "puzzlepiece.extension.fill", size: 11, weight: .semibold)
                }
                Text(installing ? "Adding…" : "Add to Mori")
                    .font(Typography.ui(Typography.small, weight: .medium))
            }
            .foregroundStyle(p.primaryForeground.color)
            .padding(.horizontal, 8)
            .frame(height: 22)
            .background(
                Capsule().fill(p.primary.color.opacity(installing ? 0.6 : 1))
            )
        }
        .buttonStyle(.plain)
        .disabled(installing)
        .help("Install this extension in Mori")
    }
}

/// The omnibox autocomplete dropdown: history matches for what's been typed.
private struct OmniboxSuggestionsList: View {
    let suggestions: [HistoryEntry]
    let highlighted: Int?
    let onPick: (HistoryEntry) -> Void

    @Environment(\.palette) private var p

    var body: some View {
        VStack(spacing: 1) {
            ForEach(Array(suggestions.enumerated()), id: \.element.id) { idx, entry in
                SuggestionRow(entry: entry, isHighlighted: idx == highlighted) {
                    onPick(entry)
                }
            }
        }
        .padding(5)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: Radius.popover, style: .continuous)
                .fill(p.popover.color)
                .shadow(color: .black.opacity(0.22), radius: 16, y: 6)
        )
        .overlay(
            RoundedRectangle(cornerRadius: Radius.popover, style: .continuous)
                .strokeBorder(p.border.color.opacity(0.6), lineWidth: 1)
        )
    }
}

private struct SuggestionRow: View {
    let entry: HistoryEntry
    let isHighlighted: Bool
    let action: () -> Void

    @Environment(\.palette) private var p
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            HStack(spacing: 9) {
                Icon(name: "clock.arrow.circlepath", size: 14, weight: .regular)
                    .foregroundStyle(p.mutedForeground.color)
                    .frame(width: 16)
                Text(entry.title.isEmpty ? entry.url : entry.title)
                    .font(Typography.ui(Typography.base))
                    .foregroundStyle(p.foreground.color)
                    .lineLimit(1)
                Spacer(minLength: 8)
                Text(prettyHost)
                    .font(Typography.ui(Typography.small))
                    .foregroundStyle(p.mutedForeground.color)
                    .lineLimit(1)
            }
            .padding(.horizontal, 9)
            .frame(height: 30)
            .background(
                RoundedRectangle(cornerRadius: Radius.md, style: .continuous)
                    .fill(isHighlighted || hovering ? p.accent.color.opacity(0.7) : .clear)
            )
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
    }

    private var prettyHost: String {
        URL(string: entry.url)?.host ?? ""
    }
}

/// A transparent AppKit view that lets you drag the window by the toolbar,
/// since the native titlebar is hidden.
struct WindowDragArea: NSViewRepresentable {
    func makeNSView(context: Context) -> NSView { DragView() }
    func updateNSView(_ nsView: NSView, context: Context) {}

    private final class DragView: NSView {
        override func mouseDown(with event: NSEvent) {
            window?.performDrag(with: event)
        }
        override var mouseDownCanMoveWindow: Bool { true }
    }
}
