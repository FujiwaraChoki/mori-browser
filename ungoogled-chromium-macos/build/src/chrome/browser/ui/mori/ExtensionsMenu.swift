import SwiftUI
import AppKit

/// The cluster shown at the trailing edge of the omnibox: pinned extensions as
/// their own icon buttons, followed by the puzzle-piece button that opens the
/// full extensions menu. Everything reflects Chrome's real extension state.
struct ExtensionToolbarItems: View {
    @ObservedObject var store: BrowserStore
    @ObservedObject private var extensions = ExtensionStore.shared
    @Environment(\.palette) private var p
    @State private var menuVisible = false

    var body: some View {
        HStack(spacing: 2) {
            ForEach(extensions.pinnedExtensions) { ext in
                ExtensionActionButton(ext: ext)
            }

            Button {
                menuVisible.toggle()
            } label: {
                Icon(name: "puzzlepiece.extension", size: 12)
                    .foregroundStyle(p.mutedForeground.color)
                    .frame(width: 20, height: 20)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help("Extensions")
            .popover(isPresented: $menuVisible, arrowEdge: .bottom) {
                ExtensionsMenu(store: store) { menuVisible = false }
            }
        }
        // Badge/title state is per-tab; keep the snapshot fresh as the user
        // switches tabs.
        .onChange(of: store.selectedTabID) { _ in extensions.refresh() }
    }
}

/// Box handing the underlying NSView to SwiftUI buttons so action popups can
/// anchor to the clicked button's true screen rect.
private final class AnchorBox {
    weak var view: NSView?

    var screenRect: NSRect {
        guard let view, let window = view.window else { return .zero }
        return window.convertToScreen(view.convert(view.bounds, to: nil))
    }
}

private struct AnchorReader: NSViewRepresentable {
    let box: AnchorBox

    func makeNSView(context: Context) -> NSView {
        let view = NSView()
        box.view = view
        return view
    }

    func updateNSView(_ nsView: NSView, context: Context) {
        box.view = nsView
    }
}

/// A pinned extension's icon button in the omnibox, with its live badge.
/// Clicking runs the real Chrome action (onClicked / popup / side panel).
private struct ExtensionActionButton: View {
    let ext: ChromeExtensionInfo
    @Environment(\.palette) private var p
    @State private var hover = false
    private let anchor = AnchorBox()

    var body: some View {
        Button {
            ExtensionStore.shared.runAction(ext, anchor: anchor.screenRect)
        } label: {
            ExtensionIconView(ext: ext, size: 16)
                .frame(width: 22, height: 22)
                .overlay(alignment: .topTrailing) { badge }
                .background(
                    RoundedRectangle(cornerRadius: 5, style: .continuous)
                        .fill(hover ? p.foreground.color.opacity(0.12) : .clear)
                )
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .background(AnchorReader(box: anchor))
        .onHover { hover = $0 }
        .help(ext.actionTitle.isEmpty ? ext.name : ext.actionTitle)
    }

    @ViewBuilder private var badge: some View {
        if !ext.badgeText.isEmpty {
            Text(String(ext.badgeText.prefix(4)))
                .font(Typography.ui(7, weight: .bold))
                .foregroundStyle(ext.badgeTextColor.color)
                .lineLimit(1)
                .minimumScaleFactor(0.6)
                .padding(.horizontal, 3)
                .frame(minWidth: 11, minHeight: 10)
                .background(Capsule().fill(ext.badgeBackgroundColor.color))
                .offset(x: 5, y: -3)
        }
    }
}

private extension Array where Element == Int {
    var color: Color {
        let r = Double(indices.contains(0) ? self[0] : 217) / 255.0
        let g = Double(indices.contains(1) ? self[1] : 48) / 255.0
        let b = Double(indices.contains(2) ? self[2] : 37) / 255.0
        let a = Double(indices.contains(3) ? self[3] : 255) / 255.0
        return Color(.sRGB, red: r, green: g, blue: b, opacity: a)
    }
}

/// Reusable square icon for an extension: its real Chrome icon, or a
/// puzzle-piece placeholder. Dimmed when the extension is disabled.
struct ExtensionIconView: View {
    let ext: ChromeExtensionInfo
    var size: CGFloat = 28
    @Environment(\.palette) private var p

    var body: some View {
        Group {
            if let icon = ext.icon {
                Image(nsImage: icon)
                    .resizable()
                    .interpolation(.high)
                    .frame(width: size, height: size)
                    .clipShape(RoundedRectangle(cornerRadius: size * 0.2, style: .continuous))
            } else {
                RoundedRectangle(cornerRadius: size * 0.2, style: .continuous)
                    .fill(p.input.color.opacity(0.6))
                    .frame(width: size, height: size)
                    .overlay(
                        Icon(name: "puzzlepiece.extension", size: size * 0.6, weight: .regular)
                            .foregroundStyle(p.mutedForeground.color)
                    )
            }
        }
        .opacity(ext.enabled ? 1 : 0.4)
    }
}

/// The popover listing every installed extension with run/pin controls and a
/// context menu mirroring Chrome's toolbar menu (options, details, manage,
/// enable/disable, remove).
struct ExtensionsMenu: View {
    @ObservedObject var store: BrowserStore
    let dismiss: () -> Void

    @ObservedObject private var extensions = ExtensionStore.shared
    @Environment(\.palette) private var p

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("Extensions")
                .font(Typography.ui(Typography.base, weight: .semibold))
                .foregroundStyle(p.foreground.color)
                .padding(.horizontal, 14)
                .padding(.top, 12)
                .padding(.bottom, 10)

            Hairline().opacity(0.6)

            if extensions.extensions.isEmpty {
                Text("No extensions installed.")
                    .font(Typography.ui(Typography.base))
                    .foregroundStyle(p.mutedForeground.color)
                    .padding(14)
            } else {
                ScrollView {
                    VStack(spacing: 1) {
                        ForEach(extensions.extensions) { ext in
                            ExtensionMenuRow(
                                ext: ext,
                                onActivate: { anchor in
                                    dismiss()
                                    extensions.runAction(ext, anchor: anchor)
                                },
                                onTogglePin: { extensions.togglePinned(ext) },
                                dismiss: dismiss
                            )
                        }
                    }
                    .padding(5)
                }
                .frame(maxHeight: 320)
            }

            Hairline().opacity(0.6)

            Button {
                extensions.openManagePage()
                dismiss()
            } label: {
                HStack(spacing: 7) {
                    Icon(name: "gearshape", size: 14, weight: .regular)
                    Text("Manage Extensions…")
                        .font(Typography.ui(Typography.base))
                    Spacer()
                }
                .foregroundStyle(p.foreground.color)
                .padding(.horizontal, 14)
                .frame(height: 38)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
        .frame(width: 300)
        .background(p.popover.color)
        .onAppear { extensions.refresh() }
    }
}

private struct ExtensionMenuRow: View {
    let ext: ChromeExtensionInfo
    let onActivate: (NSRect) -> Void
    let onTogglePin: () -> Void
    let dismiss: () -> Void

    @ObservedObject private var extensions = ExtensionStore.shared
    @Environment(\.palette) private var p
    @State private var hover = false
    private let anchor = AnchorBox()

    var body: some View {
        HStack(spacing: 10) {
            Button {
                onActivate(anchor.screenRect)
            } label: {
                HStack(spacing: 10) {
                    ExtensionIconView(ext: ext, size: 22)
                    VStack(alignment: .leading, spacing: 1) {
                        Text(ext.name)
                            .font(Typography.ui(Typography.base, weight: .medium))
                            .foregroundStyle(p.foreground.color)
                            .lineLimit(1)
                        if !ext.enabled {
                            Text("Disabled")
                                .font(Typography.ui(Typography.small))
                                .foregroundStyle(p.mutedForeground.color)
                        }
                    }
                    Spacer(minLength: 4)
                    if !ext.badgeText.isEmpty {
                        Text(String(ext.badgeText.prefix(4)))
                            .font(Typography.ui(8, weight: .bold))
                            .foregroundStyle(ext.badgeTextColor.color)
                            .padding(.horizontal, 4)
                            .frame(minHeight: 12)
                            .background(Capsule().fill(ext.badgeBackgroundColor.color))
                    }
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .disabled(!ext.enabled)
            .help(ext.enabled ? "Run \(ext.name)" : "\(ext.name) is disabled")

            Button(action: onTogglePin) {
                Icon(name: ext.pinned ? "pin.fill" : "pin", size: 14)
                    .foregroundStyle(ext.pinned ? p.primary.color : p.mutedForeground.color)
                    .frame(width: 24, height: 24)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .disabled(!ext.enabled)
            .help(ext.pinned ? "Unpin from toolbar" : "Pin to toolbar")
        }
        .padding(.horizontal, 8)
        .frame(height: 42)
        .background(
            RoundedRectangle(cornerRadius: Radius.md, style: .continuous)
                .fill(hover ? p.accent.color.opacity(0.5) : .clear)
        )
        .background(AnchorReader(box: anchor))
        .onHover { hover = $0 }
        .contextMenu {
            if ext.hasOptionsPage {
                Button("Options…") {
                    extensions.openOptions(ext)
                    dismiss()
                }
            }
            Button("Details") {
                extensions.openDetailsPage(ext)
                dismiss()
            }
            Divider()
            if ext.mayDisable {
                Button(ext.enabled ? "Disable" : "Enable") {
                    extensions.setEnabled(ext, !ext.enabled)
                }
                Button("Remove from Mori…") {
                    dismiss()
                    extensions.remove(ext)
                }
            }
        }
    }
}
