import SwiftUI

/// A folder icon that morphs between a closed and an open "pocket folder" by
/// shearing two stacked vector panels in opposite directions. Mori draws the
/// folder from simple SwiftUI shapes so the icon can inherit theme colors and
/// animate with the rest of the sidebar chrome.
///
/// A single 0→1 `progress` drives an animatable `GeometryEffect` so the back
/// and front faces splay apart together.
struct MorphingFolderIcon: View {
    /// Expanded folder == open pocket.
    var isOpen: Bool
    /// Folder is collapsed *and* holds the active tab → show dots, hide the
    /// glyph.
    var showsDots: Bool
    /// SF Symbol shown inside the front pocket.
    var symbol: String

    var size: CGFloat = 28
    var frontColor: Color
    var backColor: Color
    var stroke: Color
    var glyphColor: Color
    /// Opaque surface the icon sits on. The front pocket is filled with this so
    /// it occludes the rear panel once the folder splays open.
    var surface: Color

    /// 0 = closed, 1 = fully open. Animated; everything geometric reads from it.
    private var progress: CGFloat { isOpen ? 1 : 0 }

    var body: some View {
        ZStack {
            // Rear panel — the folder body with the little tab/flap.
            FolderBackShape()
                .fill(backColor)
                .overlay(FolderBackShape().stroke(stroke, lineWidth: 1.7 * size / 32))
                .modifier(FolderSplit(progress: progress, angleDegrees: 16,
                                      dx: -4, dy: 2, size: size))

            // Front pocket + contents — skews the opposite way and carries the glyph.
            ZStack {
                // Opaque surface base + translucent tint, so the front face hides
                // the rear panel behind it when open while keeping its tint.
                FolderFrontShape()
                    .fill(surface)
                    .overlay(FolderFrontShape().fill(frontColor))
                    .overlay(FolderFrontShape().stroke(stroke, lineWidth: 1.7 * size / 32))

                // Inner glyph ↔ dots cross-fade. The glyph only appears for a
                // *custom* folder symbol — the default "folder" would just draw
                // a folder inside the folder, so we leave the pocket empty.
                if hasCustomGlyph {
                    Icon(name: symbol, size: size * 12 / 32)
                        .foregroundStyle(glyphColor)
                        .position(x: center.x, y: center.y)
                        .opacity(showsDots ? 0 : 1)
                }

                DotsShape()
                    .fill(stroke)
                    .opacity(showsDots ? 1 : 0)
            }
            .modifier(FolderSplit(progress: progress, angleDegrees: -16,
                                  dx: 8, dy: 2, size: size))
        }
        .frame(width: size, height: size)
        .animation(.timingCurve(0.42, 0, 0, 1, duration: 0.3), value: progress)
        .animation(.easeInOut(duration: 0.3), value: showsDots)
    }

    /// A folder is "custom" when its symbol isn't the default folder glyph;
    /// only then do we draw something inside the pocket.
    private var hasCustomGlyph: Bool {
        symbol != "folder" && symbol != "folder.fill"
    }

    /// Center of the front pocket in view coordinates.
    private var center: CGPoint {
        CGPoint(x: 16 * size / 32, y: 19.5 * size / 32)
    }
}

// MARK: - Splay effect

/// Animatable shear + translate + scale about the icon center, interpolated by
/// `progress`. SwiftUI has no skew modifier, so the affine transform is exposed
/// as a `GeometryEffect`.
private struct FolderSplit: GeometryEffect {
    var progress: CGFloat
    var angleDegrees: CGFloat
    var dx: CGFloat
    var dy: CGFloat
    var size: CGFloat

    var animatableData: CGFloat {
        get { progress }
        set { progress = newValue }
    }

    func effectValue(size viewSize: CGSize) -> ProjectionTransform {
        let scale = 1 - 0.15 * progress
        let radians = angleDegrees * progress * .pi / 180
        // Translate constants are in the icon's design space; scale to points.
        let tx = dx * progress * size / 32
        let ty = dy * progress * size / 32

        let cx = viewSize.width / 2
        let cy = viewSize.height / 2

        let toPivot = CGAffineTransform(translationX: -cx, y: -cy)
        let s = CGAffineTransform(scaleX: scale, y: scale)
        let t = CGAffineTransform(translationX: tx, y: ty)
        // skewX: x' = x + tan(angle)·y
        let skew = CGAffineTransform(a: 1, b: 0, c: tan(radians), d: 1, tx: 0, ty: 0)
        let fromPivot = CGAffineTransform(translationX: cx, y: cy)

        // Applied to a point in order: -pivot → scale → translate → skew → +pivot.
        let m = toPivot
            .concatenating(s)
            .concatenating(t)
            .concatenating(skew)
            .concatenating(fromPivot)
        return ProjectionTransform(m)
    }
}

// MARK: - Vector shapes

/// Rear folder panel with the tab/flap.
private struct FolderBackShape: Shape {
    func path(in rect: CGRect) -> Path {
        let u = min(rect.width, rect.height) / 32
        func p(_ x: CGFloat, _ y: CGFloat) -> CGPoint { CGPoint(x: x * u, y: y * u) }

        var path = Path()
        path.move(to: p(7.5, 7))
        path.addLine(to: p(12.5, 7))
        path.addCurve(to: p(14.8, 8),
                      control1: p(13.4, 7), control2: p(14.2, 7.35))
        path.addLine(to: p(16.1, 9.15))
        path.addCurve(to: p(18.5, 10),
                      control1: p(16.75, 9.7), control2: p(17.55, 10))
        path.addLine(to: p(24.5, 10))
        path.addCurve(to: p(27, 12.5),
                      control1: p(25.88, 10), control2: p(27, 11.12))
        path.addLine(to: p(27, 24.5))
        path.addCurve(to: p(24.5, 27),
                      control1: p(27, 25.88), control2: p(25.88, 27))
        path.addLine(to: p(7.5, 27))
        path.addCurve(to: p(5, 24.5),
                      control1: p(6.12, 27), control2: p(5, 25.88))
        path.addLine(to: p(5, 9.5))
        path.addCurve(to: p(7.5, 7),
                      control1: p(5, 8.12), control2: p(6.12, 7))
        path.closeSubpath()
        return path
    }
}

/// Front pocket.
private struct FolderFrontShape: Shape {
    func path(in rect: CGRect) -> Path {
        let u = min(rect.width, rect.height) / 32
        let r = CGRect(x: 4.8 * u, y: 12 * u, width: 22.4 * u, height: 15 * u)
        return Path(roundedRect: r, cornerRadius: 3.2 * u, style: .continuous)
    }
}

/// Three centered dots.
private struct DotsShape: Shape {
    func path(in rect: CGRect) -> Path {
        let u = min(rect.width, rect.height) / 32
        var path = Path()
        for cx: CGFloat in [12, 16, 20] {
            let d = 2.6 * u
            path.addEllipse(in: CGRect(x: cx * u - d / 2, y: 19.5 * u - d / 2,
                                       width: d, height: d))
        }
        return path
    }
}
