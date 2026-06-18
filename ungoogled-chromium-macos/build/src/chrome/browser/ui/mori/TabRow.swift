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

    @Environment(\.palette) private var p
    @Environment(\.colorScheme) private var scheme
    @State private var hovering = false
    @State private var pressing = false
    @State private var closeHovering = false

    var body: some View {
        HStack(spacing: 9) {
            Favicon(icon: tab.faviconURL, page: tab.urlString,
                    image: tab.faviconImage,
                    size: 15,
                    active: isSelected || hovering)
                .opacity(tab.isAsleep ? 0.5 : 1)

            Text(tab.title)
                .font(Typography.ui(Typography.base))
                .foregroundStyle(isSelected ? p.sidebarForeground.color
                                            : p.sidebarForeground.color.opacity(tab.isAsleep ? 0.5 : 0.78))
                .lineLimit(1)
                .truncationMode(.tail)

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
        .contentShape(Rectangle())
        .pressShrink(perform: onSelect) { isPressing in
            pressing = isPressing
        }
        .onHover { hovering = $0 }
    }

    private var showsCloseButton: Bool {
        isSelected || hovering
    }

    private var backgroundFill: Color {
        if isSelected || (pressing && !closeHovering) {
            return TabSurface.selectedFill(scheme)
        }
        if hovering { return TabSurface.hoverFill(scheme) }
        return .clear
    }
}
