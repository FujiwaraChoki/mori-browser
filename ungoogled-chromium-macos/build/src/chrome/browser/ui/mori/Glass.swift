import SwiftUI
import AppKit

/// A behind-window vibrancy background (NSVisualEffectView). With the window set
/// non-opaque, `.behindWindow` materials sample the desktop for real
/// translucency — the foundation of the translucent sidebar / panels.
struct VisualEffectBackground: NSViewRepresentable {
    var material: NSVisualEffectView.Material
    var blending: NSVisualEffectView.BlendingMode = .behindWindow
    var emphasized: Bool = false

    func makeNSView(context: Context) -> NSVisualEffectView {
        let view = NSVisualEffectView()
        view.material = material
        view.blendingMode = blending
        view.state = .active
        view.isEmphasized = emphasized
        return view
    }

    func updateNSView(_ nsView: NSVisualEffectView, context: Context) {
        nsView.material = material
        nsView.blendingMode = blending
        nsView.isEmphasized = emphasized
    }
}

extension View {
    /// Apply Apple's native **Liquid Glass** (macOS 26) as a rounded background,
    /// falling back to an ultra-thin material on older systems.
    func liquidGlass(cornerRadius: CGFloat, interactive: Bool = false) -> some View {
        modifier(LiquidGlassModifier(cornerRadius: cornerRadius, interactive: interactive))
    }
}

struct LiquidGlassModifier: ViewModifier {
    let cornerRadius: CGFloat
    let interactive: Bool

    func body(content: Content) -> some View {
        if #available(macOS 26.0, *) {
            content.glassEffect(
                interactive ? Glass.regular.interactive() : Glass.regular,
                in: .rect(cornerRadius: cornerRadius)
            )
        } else {
            content.background(
                .ultraThinMaterial,
                in: RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
            )
        }
    }
}
