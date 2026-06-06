import SwiftUI

/// Ghost/outline icon button matching MASTER §5.1: hover/active route through a
/// translucent foreground overlay (no direct bg on ghost), color/opacity only,
/// 150ms ease, squircle-ish 10px radius. No transform-on-press.
struct IconButton: View {
    enum Kind { case ghost, outline, primary }

    let systemName: String
    var kind: Kind = .ghost
    var size: CGFloat = 28
    var disabled: Bool = false
    let action: () -> Void

    @Environment(\.palette) private var p
    @State private var hovering = false
    @State private var pressed = false

    var body: some View {
        Button(action: action) {
            Icon(name: systemName, size: 16)
                .frame(width: size, height: size)
                .foregroundStyle(foreground)
                .background(
                    RoundedRectangle(cornerRadius: Radius.button, style: .continuous)
                        .fill(background)
                )
                .overlay(
                    RoundedRectangle(cornerRadius: Radius.button, style: .continuous)
                        .strokeBorder(borderColor, lineWidth: kind == .outline ? 1 : 0)
                )
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(disabled)
        .opacity(disabled ? 0.4 : 1)
        .onHover { hovering = $0 }
        .animation(Motion.state, value: pressed)
        .simultaneousGesture(
            DragGesture(minimumDistance: 0)
                .onChanged { _ in pressed = true }
                .onEnded { _ in pressed = false }
        )
    }

    private var foreground: Color {
        switch kind {
        case .primary: return p.primaryForeground.color
        case .ghost, .outline: return p.foreground.color.opacity(0.85)
        }
    }

    private var background: Color {
        switch kind {
        case .primary:
            return p.primary.color
        case .outline:
            return p.background.color
        case .ghost:
            if pressed { return p.foreground.color.opacity(0.09) }
            if hovering { return p.foreground.color.opacity(0.05) }
            return .clear
        }
    }

    private var borderColor: Color {
        p.border.color.opacity(0.6)
    }
}

/// A favicon with Mori's compact browser styling: curated brand glyphs, a
/// domain-tinted monogram when no icon is available, and a subtle desaturation
/// when its tab is inactive. `icon` is the favicon image URL; `page` is the site
/// URL (drives brand lookup, monogram color, and the broken-image fallback).
struct Favicon: View {
    let icon: String?
    var page: String? = nil
    /// The real site favicon decoded by Chromium. When present it is preferred
    /// over the remote-URL load and the monogram (the icon/page strings still
    /// drive the brand glyph and the fallbacks).
    var image: NSImage? = nil
    var isLoading: Bool = false
    var size: CGFloat = 15
    /// Inactive tabs render slightly desaturated so the active tab reads first.
    var active: Bool = true

    private var corner: CGFloat { size * 0.27 }
    private var source: FaviconSource { FaviconSource.resolve(icon: icon, page: page) }

    var body: some View {
        ZStack {
            if isLoading {
                ProgressView()
                    .controlSize(.small)
                    .scaleEffect(0.6)
            } else {
                content
            }
        }
        .frame(width: size, height: size)
        .grayscale(active ? 0 : 0.55)
        .opacity(active ? 1 : 0.9)
        .animation(Motion.state, value: active)
    }

    /// Mori's internal pages (the new-tab page) have no real favicon; show a
    /// neutral glyph instead of a host-derived monogram.
    private var isInternal: Bool { (page ?? "").hasPrefix("mori://") }

    @ViewBuilder private var content: some View {
        if isInternal {
            Image(systemName: "magnifyingglass")
                .font(.system(size: size * 0.72, weight: .medium))
                .foregroundStyle(.secondary)
                .frame(width: size, height: size)
        } else {
            resolvedContent
        }
    }

    @ViewBuilder private var resolvedContent: some View {
        if case .brand(let asset) = source {
            Image(asset)
                .resizable()
                .interpolation(.high)
                .frame(width: size, height: size)
                .clipShape(RoundedRectangle(cornerRadius: corner, style: .continuous))
        } else if let image {
            // The actual site favicon Chromium downloaded and decoded.
            Image(nsImage: image)
                .resizable()
                .interpolation(.high)
                .frame(width: size, height: size)
                .clipShape(RoundedRectangle(cornerRadius: corner, style: .continuous))
        } else {
            switch source {
            case .remote(let url):
                AsyncImage(url: url) { phase in
                    switch phase {
                    case .success(let image):
                        image.resizable().interpolation(.high)
                    default:
                        monogram   // broken/missing → monogram, not a blank box
                    }
                }
                .frame(width: size, height: size)
                .clipShape(RoundedRectangle(cornerRadius: corner, style: .continuous))
            case .monogram(let letter, let color):
                FaviconMonogram(letter: letter, color: color, size: size)
            case .brand:
                EmptyView()  // handled above
            }
        }
    }

    /// Monogram for the broken-image case, derived from the page host.
    private var monogram: some View {
        let m = FaviconSource.monogram(for: FaviconSource.host(from: page))
        if case .monogram(let letter, let color) = m {
            return AnyView(FaviconMonogram(letter: letter, color: color, size: size))
        }
        return AnyView(Color.clear)
    }
}

/// Hairline divider using the border token.
struct Hairline: View {
    var vertical = false
    @Environment(\.palette) private var p
    var body: some View {
        Rectangle()
            .fill(p.border.color.opacity(0.6))
            .frame(width: vertical ? 1 : nil, height: vertical ? nil : 1)
    }
}
