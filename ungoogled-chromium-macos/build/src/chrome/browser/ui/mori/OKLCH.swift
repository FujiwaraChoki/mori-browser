import SwiftUI

/// Color-space helpers so we can transcribe Mori's `globals.css` tokens
/// verbatim (many are authored in `oklch(...)`) and resolve them to sRGB with
/// full fidelity instead of eyeballing hex equivalents.
enum ColorMath {
    /// Convert `oklch(L C h)` (L in 0…1, C chroma, h in degrees) to sRGB
    /// components in 0…1. Mirrors the CSS Color 4 reference conversion:
    /// OKLCH → OKLab → linear-sRGB → gamma-encoded sRGB.
    static func oklchToSRGB(L: Double, C: Double, h: Double) -> (r: Double, g: Double, b: Double) {
        let hr = h * .pi / 180.0
        let a = C * cos(hr)
        let bb = C * sin(hr)

        // OKLab -> LMS (nonlinear)
        let l_ = L + 0.3963377774 * a + 0.2158037573 * bb
        let m_ = L - 0.1055613458 * a - 0.0638541728 * bb
        let s_ = L - 0.0894841775 * a - 1.2914855480 * bb

        let l = l_ * l_ * l_
        let m = m_ * m_ * m_
        let s = s_ * s_ * s_

        // LMS -> linear sRGB
        let rLin = 4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s
        let gLin = -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s
        let bLin = -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s

        return (gammaEncode(rLin), gammaEncode(gLin), gammaEncode(bLin))
    }

    /// linear-sRGB component -> gamma-encoded sRGB, clamped to display range.
    private static func gammaEncode(_ c: Double) -> Double {
        let clamped = min(max(c, 0.0), 1.0)
        if clamped <= 0.0031308 {
            return 12.92 * clamped
        }
        return 1.055 * pow(clamped, 1.0 / 2.4) - 0.055
    }
}

/// A theme color authored either as hex or OKLCH, with optional alpha — exactly
/// the two shapes used across `globals.css`.
struct TokenColor {
    let r: Double
    let g: Double
    let b: Double
    let a: Double

    init(r: Double, g: Double, b: Double, a: Double = 1) {
        self.r = r; self.g = g; self.b = b; self.a = a
    }

    /// `#rgb`, `#rrggbb`, or `#rrggbbaa`.
    init(hex: String, alpha: Double = 1) {
        var s = hex.trimmingCharacters(in: .whitespaces)
        if s.hasPrefix("#") { s.removeFirst() }
        if s.count == 3 {
            s = s.map { "\($0)\($0)" }.joined()
        }
        var value: UInt64 = 0
        Scanner(string: s).scanHexInt64(&value)
        let hasAlpha = s.count == 8
        if hasAlpha {
            r = Double((value >> 24) & 0xFF) / 255
            g = Double((value >> 16) & 0xFF) / 255
            b = Double((value >> 8) & 0xFF) / 255
            a = Double(value & 0xFF) / 255
        } else {
            r = Double((value >> 16) & 0xFF) / 255
            g = Double((value >> 8) & 0xFF) / 255
            b = Double(value & 0xFF) / 255
            a = alpha
        }
    }

    /// `oklch(L C h)` with optional alpha.
    init(L: Double, C: Double, h: Double, alpha: Double = 1) {
        let rgb = ColorMath.oklchToSRGB(L: L, C: C, h: h)
        r = rgb.r; g = rgb.g; b = rgb.b; a = alpha
    }

    var color: Color {
        Color(.sRGB, red: r, green: g, blue: b, opacity: a)
    }

    /// `#rrggbb` (alpha dropped). Round-trips with `init(hex:)` for the 0…255
    /// values a swatch or color well produces, so it's safe to key selection on.
    var hexString: String {
        func byte(_ c: Double) -> Int { Int((min(max(c, 0), 1) * 255).rounded()) }
        return String(format: "#%02X%02X%02X", byte(r), byte(g), byte(b))
    }

    var nsColor: NSColor {
        NSColor(srgbRed: r, green: g, blue: b, alpha: a)
    }

    func opacity(_ value: Double) -> TokenColor {
        TokenColor(r: r, g: g, b: b, a: value)
    }
}

extension TokenColor {
    static func hex(_ h: String, _ alpha: Double = 1) -> TokenColor { TokenColor(hex: h, alpha: alpha) }
    static func oklch(_ L: Double, _ C: Double, _ h: Double, _ alpha: Double = 1) -> TokenColor {
        TokenColor(L: L, C: C, h: h, alpha: alpha)
    }
}
