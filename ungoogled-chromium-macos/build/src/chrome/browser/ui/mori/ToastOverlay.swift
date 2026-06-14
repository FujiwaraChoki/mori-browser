import SwiftUI

/// Renders the live toast queue from `ToastCenter` as a bottom-centered stack of
/// pills. Purely presentational and stateless beyond the observed center, so any
/// feature can raise a notification with `ToastCenter.shared.show(...)` without
/// touching this view. Mount it as a full-window overlay above the web content.
struct ToastOverlay: View {
    @ObservedObject var center: ToastCenter

    var body: some View {
        VStack(spacing: 8) {
            ForEach(center.toasts) { toast in
                ToastView(toast: toast) { center.dismiss(toast.id) }
                    .transition(.move(edge: .bottom).combined(with: .opacity))
            }
        }
        .padding(.bottom, 24)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottom)
        // Only the pills should be interactive; the rest of the overlay must let
        // clicks fall through to the page behind it.
        .allowsHitTesting(false)
        .animation(Motion.snappy, value: center.toasts)
    }
}

/// One toast pill: a translucent, shadowed capsule with an optional accent icon.
private struct ToastView: View {
    let toast: Toast
    let onDismiss: () -> Void

    @Environment(\.palette) private var p
    @State private var hovering = false

    var body: some View {
        HStack(spacing: 9) {
            if let icon = toast.icon {
                Icon(name: icon, size: 14)
                    .foregroundStyle(accent)
            }
            Text(toast.message)
                .font(Typography.ui(Typography.base, weight: .medium))
                .foregroundStyle(p.foreground.color)
                .lineLimit(2)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
        .background(
            ZStack {
                VisualEffectBackground(material: .popover)
                p.popover.color.opacity(0.55)
            }
            .clipShape(RoundedRectangle(cornerRadius: Radius.popover, style: .continuous))
        )
        .overlay(
            RoundedRectangle(cornerRadius: Radius.popover, style: .continuous)
                .strokeBorder(p.border.color.opacity(0.6), lineWidth: 1)
        )
        .shadow(color: .black.opacity(0.20), radius: 12, x: 0, y: 4)
        .scaleEffect(hovering ? 1.02 : 1)
        .contentShape(RoundedRectangle(cornerRadius: Radius.popover, style: .continuous))
        .onHover { hovering = $0 }
        .onTapGesture { onDismiss() }
        // Re-enable hit testing on the pill itself (the container disables it so
        // empty space stays click-through).
        .allowsHitTesting(true)
    }

    private var accent: Color {
        switch toast.style {
        case .info: return p.statusInfoFg.color
        case .success: return p.statusSuccessFg.color
        case .warning: return p.statusWarningFg.color
        case .error: return p.destructive.color
        }
    }
}
