import SwiftUI

/// A single vertical tab row: a selected tab is a translucent white fill lifted
/// by a soft shadow (no border), at rest it is transparent, hover is a quiet
/// overlay. Close button reveals on hover.
///
/// Selection uses a plain `.onTapGesture` rather than a `Button` or a
/// `DragGesture`-based press effect on purpose: the sidebar attaches `.onDrag`
/// to this row, and a `DragGesture(minimumDistance:)` (or, on some macOS
/// versions, a `Button`) claims the pointer first and stops SwiftUI's `.onDrag`
/// from ever starting a drag session — which is what broke sidebar
/// drag-and-drop. A tap gesture coexists cleanly with `.onDrag`.
struct TabRow: View {
    @ObservedObject var tab: BrowserTab
    let isSelected: Bool
    let onSelect: () -> Void
    let onClose: () -> Void
    /// True when the store has flagged this row to enter inline rename.
    var pendingRename: Bool = false
    /// True when this row is part of a ⌘/⇧-click multi-selection.
    var isMultiSelected: Bool = false
    /// Commit a new custom title (empty clears the override).
    var onRename: (String) -> Void = { _ in }
    /// Tell the store the pending-rename request has been consumed.
    var onRenameConsumed: () -> Void = {}

    @Environment(\.palette) private var p
    @Environment(\.colorScheme) private var scheme
    @State private var hovering = false
    @State private var pressing = false
    @State private var closeHovering = false
    @State private var isEditing = false
    @State private var draftName = ""
    @FocusState private var nameFocused: Bool

    var body: some View {
        HStack(spacing: 9) {
            if tab.kind == .agent, let agent = tab.agent {
                AgentTabGlyph(assistant: agent)
            } else {
                Favicon(icon: tab.faviconURL, page: tab.urlString,
                        image: tab.faviconImage,
                        size: 15,
                        active: isSelected || hovering)
                    .opacity(tab.isAsleep ? 0.5 : 1)
            }

            if isEditing {
                TextField("Tab name", text: $draftName)
                    .textFieldStyle(.plain)
                    .font(Typography.ui(Typography.base))
                    .foregroundStyle(p.sidebarForeground.color)
                    .focused($nameFocused)
                    .onSubmit(commitRename)
                    .onChange(of: nameFocused) { _, focused in
                        if !focused { commitRename() }
                    }
                    .onKeyPress(.escape) {
                        isEditing = false
                        return .handled
                    }
            } else {
                Text(tab.displayTitle)
                    .font(Typography.ui(Typography.base))
                    .foregroundStyle(isSelected ? p.sidebarForeground.color
                                                : p.sidebarForeground.color.opacity(tab.isAsleep ? 0.5 : 0.78))
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .help(tab.displayTitle)
            }

            Spacer(minLength: 0)

            // Audio indicator / mute toggle for tabs that are (or were) playing.
            if tab.isAudible || tab.isMuted {
                Button { tab.toggleMute() } label: {
                    Icon(name: tab.isMuted ? "speaker.slash.fill" : "speaker.wave.2.fill", size: 11)
                        .foregroundStyle(tab.isMuted ? p.mutedForeground.color
                                                     : p.sidebarForeground.color.opacity(0.75))
                        .frame(width: 18, height: 18)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .help(tab.isMuted ? "Unmute tab" : "Mute tab")
            }

            Button(action: onClose) {
                Icon(name: "xmark", size: 11, weight: .bold)
                    .foregroundStyle(p.mutedForeground.color)
                    .frame(width: 18, height: 18)
                    .background(
                        RoundedRectangle(cornerRadius: Radius.md, style: .continuous)
                            .fill(closeHovering ? p.sidebarForeground.color.opacity(0.10) : .clear)
                    )
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .onHover { closeHovering = $0 }
            .help("Close tab")
            .opacity(showsCloseButton ? 1 : 0)
            .allowsHitTesting(showsCloseButton)
            .accessibilityHidden(!showsCloseButton)
        }
        .padding(.leading, 9)
        // The xmark asset carries ~3pt of its own trailing whitespace, so a
        // smaller pad here lands the glyph the same ~9pt from the card edge as
        // the favicon sits from the leading edge.
        .padding(.trailing, 6)
        .frame(height: 34)
        .background(
            RoundedRectangle(cornerRadius: TabSurface.radius, style: .continuous)
                .fill(backgroundFill)
                .shadow(color: isSelected ? TabSurface.shadow(scheme) : .clear,
                        radius: isSelected ? TabSurface.shadowRadius : 0,
                        x: 0, y: isSelected ? TabSurface.shadowY : 0)
                .transaction { transaction in
                    transaction.animation = nil
                }
        )
        // Multi-selection ring (⌘/⇧-click) so a batch reads as a group.
        .overlay(
            RoundedRectangle(cornerRadius: TabSurface.radius, style: .continuous)
                .strokeBorder(p.primary.color.opacity(isMultiSelected ? 0.8 : 0), lineWidth: 1.5)
        )
        .contentShape(Rectangle())
        .pressShrink(perform: { if !isEditing { onSelect() } }) { isPressing in
            pressing = isPressing
        }
        .onHover { hovering = $0 }
        .onMiddleClick { if !isEditing { onClose() } }
        .onAppear { if pendingRename { beginRename() } }
        .onChange(of: pendingRename) { _, now in
            if now { beginRename() }
        }
    }

    private var showsCloseButton: Bool {
        (isSelected || hovering) && !isEditing
    }

    private func beginRename() {
        draftName = tab.displayTitle
        isEditing = true
        DispatchQueue.main.async { nameFocused = true }
        onRenameConsumed()
    }

    private func commitRename() {
        guard isEditing else { return }
        onRename(draftName)
        isEditing = false
    }

    private var backgroundFill: Color {
        if isSelected || (pressing && !closeHovering) {
            return TabSurface.selectedFill(scheme)
        }
        if hovering { return TabSurface.hoverFill(scheme) }
        return .clear
    }
}

/// Leading glyph for an agent tab: a sparkles mark that becomes a spinner while
/// the agent is working. Observes the tab's assistant directly (a separate
/// ObservableObject from the tab) so the working state stays live in the row.
private struct AgentTabGlyph: View {
    @ObservedObject var assistant: CodexBrowserAssistant
    @Environment(\.palette) private var p

    var body: some View {
        ZStack {
            if assistant.isWorking {
                ProgressView()
                    .controlSize(.small)
                    .scaleEffect(0.55)
            } else {
                Icon(name: "sparkles", size: 15)
                    .foregroundStyle(p.accent.color)
            }
        }
        .frame(width: 15, height: 15)
    }
}
