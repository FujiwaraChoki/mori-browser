import SwiftUI

/// A Chrome/Safari-style find-in-page bar that floats at the top-right of the
/// web card. Cmd-F shows it; Return / Shift-Return cycle matches; Esc closes.
struct FindBar: View {
    @ObservedObject var store: BrowserStore
    @ObservedObject var tab: BrowserTab
    @Environment(\.palette) private var p

    @FocusState private var focused: Bool

    var body: some View {
        HStack(spacing: 8) {
            Icon(name: "magnifyingglass", size: 14)
                .foregroundStyle(p.mutedForeground.color)

            TextField("Find on page", text: $store.findQuery)
                .textFieldStyle(.plain)
                .font(Typography.ui(Typography.base))
                .foregroundStyle(p.foreground.color)
                .frame(width: 180)
                .focused($focused)
                .onSubmit { search(forward: true) }
                .onChange(of: store.findQuery) { _, text in
                    if text.isEmpty { tab.stopFind() } else { tab.find(text, forward: true) }
                }

            Text(matchLabel)
                .font(Typography.ui(Typography.small))
                .foregroundStyle(p.mutedForeground.color)
                .frame(minWidth: 44, alignment: .trailing)
                .monospacedDigit()

            Hairline(vertical: true).frame(height: 18).opacity(0.5)

            IconButton(systemName: "chevron.up", size: 24,
                       disabled: tab.findCount == 0) { search(forward: false) }
            IconButton(systemName: "chevron.down", size: 24,
                       disabled: tab.findCount == 0) { search(forward: true) }
            IconButton(systemName: "xmark", size: 24) { store.hideFindBar() }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .background(
            RoundedRectangle(cornerRadius: Radius.popover, style: .continuous)
                .fill(p.popover.color)
                .shadow(color: .black.opacity(0.18), radius: 12, y: 4)
        )
        .overlay(
            RoundedRectangle(cornerRadius: Radius.popover, style: .continuous)
                .strokeBorder(p.border.color.opacity(0.6), lineWidth: 1)
        )
        .padding(.top, 10)
        .padding(.trailing, 14)
        .onAppear {
            focused = true
            if !store.findQuery.isEmpty { tab.find(store.findQuery, forward: true) }
        }
        // Re-focus and re-run when reopened against a different tab.
        .onChange(of: store.findBarVisible) { _, visible in
            if visible { focused = true }
        }
    }

    private var matchLabel: String {
        guard !store.findQuery.isEmpty else { return "" }
        if tab.findCount == 0 { return "No results" }
        return "\(tab.findOrdinal)/\(tab.findCount)"
    }

    private func search(forward: Bool) {
        guard !store.findQuery.isEmpty else { return }
        tab.find(store.findQuery, forward: forward)
    }
}
