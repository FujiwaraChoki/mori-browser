import SwiftUI

/// Pure color math for the gradient theme: mapping points on the circular picker
/// to colors and back, deriving harmonized dots, building the chrome-wash
/// gradient, deriving a legible UI accent, and resolving auto light/dark from
/// the gradient's luminance. Stateless — mirrors the `ColorMath` namespace.
///
/// Angle convention (must match the wheel's `AngularGradient`): hue 0° (red)
/// sits at 3 o'clock and increases clockwise. SwiftUI's y-axis grows downward,
/// so `atan2(dy, dx)` already reads clockwise on screen — no sign flip needed.
enum GradientEngine {

    // MARK: HSL ↔ RGB

    /// `h` in 0…360, `s`/`l` in 0…1.
    static func hslToRGB(h: Double, s: Double, l: Double) -> RGB {
        let hue = (h.truncatingRemainder(dividingBy: 360) + 360)
            .truncatingRemainder(dividingBy: 360)
        let c = (1 - abs(2 * l - 1)) * s
        let x = c * (1 - abs((hue / 60).truncatingRemainder(dividingBy: 2) - 1))
        let m = l - c / 2
        let (r1, g1, b1): (Double, Double, Double)
        switch hue {
        case   0..<60:  (r1, g1, b1) = (c, x, 0)
        case  60..<120: (r1, g1, b1) = (x, c, 0)
        case 120..<180: (r1, g1, b1) = (0, c, x)
        case 180..<240: (r1, g1, b1) = (0, x, c)
        case 240..<300: (r1, g1, b1) = (x, 0, c)
        default:        (r1, g1, b1) = (c, 0, x)
        }
        return RGB(r: r1 + m, g: g1 + m, b: b1 + m)
    }

    static func rgbToHSL(_ rgb: RGB) -> (h: Double, s: Double, l: Double) {
        let r = rgb.r, g = rgb.g, b = rgb.b
        let maxV = max(r, g, b), minV = min(r, g, b)
        let delta = maxV - minV
        let l = (maxV + minV) / 2

        guard delta > 0 else { return (0, 0, l) }

        let s = delta / (1 - abs(2 * l - 1))
        var h: Double
        if maxV == r {
            h = 60 * (((g - b) / delta).truncatingRemainder(dividingBy: 6))
        } else if maxV == g {
            h = 60 * ((b - r) / delta + 2)
        } else {
            h = 60 * ((r - g) / delta + 4)
        }
        if h < 0 { h += 360 }
        return (h, s, l)
    }

    // MARK: Wheel position ↔ color

    /// Map a normalized point (center `0.5, 0.5`) to a color: radius → saturation,
    /// angle → hue, with an optional explicit lightness (0…100).
    static func colorFromPosition(_ pos: CGPoint, lightness: Double?) -> RGB {
        let dx = Double(pos.x) - 0.5
        let dy = Double(pos.y) - 0.5
        let saturation = min(1, hypot(dx, dy) / 0.5)
        var angle = atan2(dy, dx) * 180 / .pi
        if angle < 0 { angle += 360 }
        let l = (lightness ?? 50) / 100
        return hslToRGB(h: angle, s: saturation, l: l)
    }

    /// Inverse: place a known color back on the wheel (used by swatch / hex).
    static func positionFromColor(_ rgb: RGB) -> (position: CGPoint, lightness: Double) {
        let (h, s, l) = rgbToHSL(rgb)
        let rad = h * .pi / 180
        let radius = min(1, s) * 0.5
        let pos = CGPoint(x: 0.5 + cos(rad) * radius,
                          y: 0.5 + sin(rad) * radius)
        return (pos, l * 100)
    }

    // MARK: Harmonies

    /// Regenerate the secondary dots for an algorithm from the primary's hue.
    /// `floating` yields none — dots are placed freely.
    static func harmonyDots(primary: GradientDot, algorithm: HarmonyAlgorithm) -> [GradientDot] {
        let (h, s, l) = rgbToHSL(primary.rgb)
        return algorithm.offsets.map { offset in
            let rgb = hslToRGB(h: h + offset, s: s, l: l)
            let (pos, light) = positionFromColor(rgb)
            return GradientDot(rgb: rgb, x: Double(pos.x), y: Double(pos.y),
                               lightness: light, algorithm: algorithm,
                               isPrimary: false, isCustom: primary.isCustom)
        }
    }

    // MARK: Chrome surface

    /// The chrome-wash fill for a theme: a soft, blurred radial mesh of the
    /// theme's dot colors (see `GradientMesh`). One dot reads as a solid tint;
    /// two or three blend into a smooth wash with no hard seam.
    @ViewBuilder
    static func chromeView(for theme: GradientTheme, scheme: ColorScheme) -> some View {
        let colors = theme.dots.map { $0.rgb.color }
        if colors.isEmpty {
            Color.clear
        } else {
            GradientMesh(colors: colors)
        }
    }

    // MARK: Accent derivation

    /// A UI accent derived from the primary dot, with saturation/lightness nudged
    /// so it stays legible against the chrome in the given scheme.
    static func accentForUI(_ theme: GradientTheme, scheme: ColorScheme) -> RGB {
        guard let primary = theme.primaryDot else { return RGB(r: 0.5, g: 0.5, b: 0.5) }
        var (h, s, l) = rgbToHSL(primary.rgb)
        if scheme == .dark {
            s = min(1, s * 1.15)
            l = max(l, 0.6)
        } else {
            s = min(1, s * 1.08)
            l = min(max(l, 0.4), 0.55)
        }
        return hslToRGB(h: h, s: s, l: l)
    }

    // MARK: Luminance / contrast

    /// WCAG relative luminance (0 = black, 1 = white).
    static func relativeLuminance(_ rgb: RGB) -> Double {
        func lin(_ c: Double) -> Double {
            c <= 0.03928 ? c / 12.92 : pow((c + 0.055) / 1.055, 2.4)
        }
        return 0.2126 * lin(rgb.r) + 0.7152 * lin(rgb.g) + 0.0722 * lin(rgb.b)
    }

    static func isDark(_ rgb: RGB) -> Bool { relativeLuminance(rgb) < 0.4 }

    /// A legible text/foreground color to lay over `rgb`.
    static func contrastingText(on rgb: RGB) -> RGB {
        isDark(rgb) ? RGB(r: 0.96, g: 0.96, b: 0.96) : RGB(r: 0.12, g: 0.12, b: 0.13)
    }

    /// Resolve the effective scheme: honor an explicit override, else infer from
    /// the average luminance of the theme's dots.
    static func effectiveScheme(for theme: GradientTheme, base: ColorScheme) -> ColorScheme {
        switch theme.schemeOverride {
        case .light: return .light
        case .dark:  return .dark
        case .auto:
            guard !theme.isEmpty else { return base }
            let avg = theme.dots.map { relativeLuminance($0.rgb) }.reduce(0, +) / Double(theme.dots.count)
            return avg < 0.4 ? .dark : .light
        }
    }
}

/// A soft, blurred radial mesh of colors. Each color blooms from a corner over
/// an opaque base of the first color; the stack is overscanned and blurred so
/// the transitions read as a smooth wash with no hard diagonal seam. Callers
/// clip the overflow. Used for both the chrome wash and the gallery tiles, so
/// the blur scales with size (capped, to keep full-window use cheap).
struct GradientMesh: View {
    let colors: [Color]
    /// Blur as a fraction of the larger dimension.
    var relativeBlur: CGFloat = 0.14
    /// Upper bound on the blur so window-scale washes don't get pathological.
    var maxBlur: CGFloat = 70

    var body: some View {
        GeometryReader { geo in
            let maxDim = max(geo.size.width, geo.size.height)
            let reach = maxDim * 1.1
            ZStack {
                // Opaque base keeps edges solid so the blur can't fade them out.
                (colors.first ?? Color.clear)
                if colors.count >= 2 {
                    bloom(colors[1], at: .bottomTrailing, reach: reach)
                }
                if colors.count >= 3 {
                    bloom(colors[2], at: .topTrailing, reach: reach)
                    // Re-anchor the primary so it still reads at top-leading.
                    bloom(colors[0], at: .topLeading, reach: reach * 0.9)
                }
            }
            .scaleEffect(1.35)   // overscan so the blur has bleed to sample
            .blur(radius: min(maxDim * relativeBlur, maxBlur))
        }
    }

    private func bloom(_ color: Color, at point: UnitPoint, reach: CGFloat) -> some View {
        RadialGradient(colors: [color, color.opacity(0)],
                       center: point, startRadius: 0, endRadius: reach)
    }
}
