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
    @Environment(\.colorScheme) private var scheme
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
        .elevation(.popover, scheme)
        .scaleEffect(hovering ? 1.02 : 1)
        .animation(Motion.state, value: hovering)
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

struct PermissionPromptOverlay: View {
    @ObservedObject var center: PermissionPromptCenter

    var body: some View {
        VStack(alignment: .trailing, spacing: 10) {
            ForEach(center.prompts) { prompt in
                PermissionPromptCard(prompt: prompt, center: center)
                    .transition(.move(edge: .top).combined(with: .opacity))
            }
        }
        .padding(.top, 54)
        .padding(.trailing, 18)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topTrailing)
        // No container-level hit-test gate. The frame draws nothing, so empty
        // space already falls through to the page; the cards are real content
        // and stay clickable. Disabling hit testing here would also disable it
        // on the cards — a parent's `false` can't be re-enabled by a child —
        // which is exactly what swallowed the Allow/Block taps.
        .animation(Motion.snappy, value: center.prompts)
    }
}

private struct PermissionPromptCard: View {
    let prompt: PermissionPromptItem
    let center: PermissionPromptCenter

    @Environment(\.palette) private var p
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(alignment: .top, spacing: 10) {
                Icon(name: "lock.fill", size: 15)
                    .foregroundStyle(p.statusInfoFg.color)
                    .frame(width: 22, height: 22)
                    .background(
                        Circle()
                            .fill(p.statusInfoFg.color.opacity(scheme == .dark ? 0.18 : 0.12))
                    )

                VStack(alignment: .leading, spacing: 6) {
                    Text("\(prompt.origin) wants access")
                        .font(Typography.ui(Typography.base, weight: .semibold))
                        .foregroundStyle(p.foreground.color)
                        .lineLimit(2)
                        .fixedSize(horizontal: false, vertical: true)

                    VStack(alignment: .leading, spacing: 4) {
                        ForEach(prompt.requests, id: \.self) { request in
                            HStack(alignment: .firstTextBaseline, spacing: 6) {
                                Circle()
                                    .fill(p.mutedForeground.color.opacity(0.8))
                                    .frame(width: 4, height: 4)
                                Text(request)
                                    .font(Typography.ui(Typography.label))
                                    .foregroundStyle(p.mutedForeground.color)
                                    .lineLimit(2)
                                    .fixedSize(horizontal: false, vertical: true)
                            }
                        }
                    }
                }
            }

            HStack(spacing: 8) {
                PromptButton("Not Now") {
                    center.respond(to: prompt, with: .dismiss)
                }
                PromptButton("Block") {
                    center.respond(to: prompt, with: .block)
                }
                PromptButton("Allow", primary: true) {
                    center.respond(to: prompt, with: .allow)
                }
            }
        }
        .padding(16)
        .frame(width: 332, alignment: .leading)
        // Solid popover surface, matching Mori's native menus (WebContextMenuCard)
        // — not the translucent vibrancy used for transient toasts. A permission
        // request is a decision surface and should read as opaque, not glass.
        .background(
            RoundedRectangle(cornerRadius: Radius.popover, style: .continuous)
                .fill(p.popover.color)
        )
        .overlay(
            RoundedRectangle(cornerRadius: Radius.popover, style: .continuous)
                .strokeBorder(p.border.color.opacity(Stroke.border), lineWidth: 1)
        )
        .elevation(.popover, scheme)
        .contentShape(RoundedRectangle(cornerRadius: Radius.popover, style: .continuous))
    }
}

private struct PromptButton: View {
    let title: String
    var primary = false
    let action: () -> Void

    @Environment(\.palette) private var p
    @State private var hovering = false

    init(_ title: String, primary: Bool = false, action: @escaping () -> Void) {
        self.title = title
        self.primary = primary
        self.action = action
    }

    var body: some View {
        Button(action: action) {
            Text(title)
                .font(Typography.ui(Typography.label, weight: .medium))
                .foregroundStyle(primary ? p.primaryForeground.color : p.foreground.color)
                .lineLimit(1)
                .frame(maxWidth: .infinity)
                .frame(height: 28)
                .background(
                    RoundedRectangle(cornerRadius: Radius.button, style: .continuous)
                        .fill(background)
                )
                .overlay(
                    RoundedRectangle(cornerRadius: Radius.button, style: .continuous)
                        .strokeBorder(border, lineWidth: primary ? 0 : 1)
                )
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .animation(Motion.state, value: hovering)
    }

    private var background: Color {
        if primary {
            return hovering ? p.primary.color.opacity(0.9) : p.primary.color
        }
        return hovering ? p.accent.color : p.muted.color
    }

    private var border: Color {
        p.border.color.opacity(Stroke.border)
    }
}
