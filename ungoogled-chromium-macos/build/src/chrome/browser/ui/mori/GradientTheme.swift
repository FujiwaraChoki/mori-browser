import SwiftUI

/// A user-chosen color theme that washes the browser chrome in a gradient and
/// derives a matching UI accent — the signature gradient-picker experience.
///
/// A theme is a small set of color *dots* placed on a circular hue/saturation
/// wheel (up to three), plus a chrome opacity, a grain-texture amount, and a
/// light/dark scheme override. Empty == "no custom theme" (chrome falls back to
/// the plain light/dark palette tint). `Codable` so it round-trips through
/// `UserDefaults` as JSON.

/// An sRGB triple in 0…1. The persisted source of truth for a dot's color, and
/// the bridge to the existing `TokenColor` / SwiftUI `Color`.
struct RGB: Codable, Equatable {
    var r: Double
    var g: Double
    var b: Double

    var token: TokenColor { TokenColor(r: r, g: g, b: b, a: 1) }
    var color: Color { token.color }

    init(r: Double, g: Double, b: Double) {
        self.r = r; self.g = g; self.b = b
    }

    /// Bridge from an existing `TokenColor` (drops alpha).
    init(_ token: TokenColor) {
        r = token.r; g = token.g; b = token.b
    }

    /// Bridge from a SwiftUI `Color` (e.g. a native color well), resolved in
    /// sRGB. Falls back to mid-grey if the color can't be resolved.
    init(_ color: Color) {
        let ns = NSColor(color).usingColorSpace(.sRGB) ?? NSColor(srgbRed: 0.5, green: 0.5, blue: 0.5, alpha: 1)
        r = Double(ns.redComponent); g = Double(ns.greenComponent); b = Double(ns.blueComponent)
    }
}

/// How the secondary dots are derived from the primary dot's hue. Mirrors the
/// color-harmony rotations in the reference picker.
enum HarmonyAlgorithm: String, Codable, CaseIterable, Identifiable {
    case floating
    case complementary
    case singleAnalogous
    case splitComplementary
    case analogous
    case triadic

    var id: String { rawValue }

    /// Hue offsets (degrees) of the auto-generated secondary dots. `floating`
    /// generates none — every dot is placed freely.
    var offsets: [Double] {
        switch self {
        case .floating:           return []
        case .complementary:      return [180]
        case .singleAnalogous:    return [310]
        case .splitComplementary: return [150, 210]
        case .analogous:          return [50, 310]
        case .triadic:            return [120, 240]
        }
    }

    /// The number of dots this algorithm implies (primary + secondaries).
    var dotCount: Int { offsets.count + 1 }

    var label: String {
        switch self {
        case .floating:           return "Free"
        case .complementary:      return "Complementary"
        case .singleAnalogous:    return "Analogous"
        case .splitComplementary: return "Split"
        case .analogous:          return "Analogous ×2"
        case .triadic:            return "Triadic"
        }
    }
}

/// One color dot on the circular picker.
///
/// `x`/`y` are normalized to 0…1 within the wheel (center `0.5, 0.5`) so the
/// model is resolution-independent and survives wheel resizes. Stored as
/// `Double` rather than `CGPoint` to keep `Codable` simple and precise.
struct GradientDot: Codable, Equatable, Identifiable {
    var id: UUID
    var rgb: RGB
    var x: Double
    var y: Double
    /// Explicit lightness override 0…100; `nil` derives from the wheel plane.
    var lightness: Double?
    var algorithm: HarmonyAlgorithm
    var isPrimary: Bool
    /// Set when entered via hex / swatch rather than dragged on the wheel.
    var isCustom: Bool

    init(id: UUID = UUID(),
         rgb: RGB,
         x: Double,
         y: Double,
         lightness: Double? = nil,
         algorithm: HarmonyAlgorithm = .floating,
         isPrimary: Bool = false,
         isCustom: Bool = false) {
        self.id = id
        self.rgb = rgb
        self.x = x
        self.y = y
        self.lightness = lightness
        self.algorithm = algorithm
        self.isPrimary = isPrimary
        self.isCustom = isCustom
    }

    var position: CGPoint {
        get { CGPoint(x: x, y: y) }
        set { x = newValue.x; y = newValue.y }
    }
}

/// The full theme: 1…3 dots plus chrome opacity, grain texture, and a
/// light/dark override.
struct GradientTheme: Codable, Equatable {
    var dots: [GradientDot]
    /// Strength of the chrome wash (matches the reference macOS range).
    var opacity: Double
    /// Grain/film-texture amount, 0…1.
    var texture: Double
    var schemeOverride: SchemeMode
    /// The identifier of the preset this theme came from, if any. Lets the
    /// gallery highlight the active preset without comparing dot UUIDs. `nil`
    /// for "no theme" or a hand-built theme. Optional + defaulted so older
    /// persisted JSON (without this key) still decodes.
    var presetID: String? = nil

    enum SchemeMode: String, Codable, CaseIterable, Identifiable {
        case auto, light, dark
        var id: String { rawValue }

        var label: String {
            switch self {
            case .auto:  return "Auto"
            case .light: return "Light"
            case .dark:  return "Dark"
            }
        }

        var symbol: String {
            switch self {
            case .auto:  return "circle.lefthalf.filled"
            case .light: return "sun.max"
            case .dark:  return "moon"
            }
        }
    }

    /// Chrome wash opacity bounds (the reference picker clamps macOS to this).
    static let minOpacity: Double = 0.3
    static let maxOpacity: Double = 0.9

    /// The dot driving harmony derivation: the flagged primary, else the first.
    var primaryDot: GradientDot? {
        dots.first(where: \.isPrimary) ?? dots.first
    }

    /// "No custom theme" — chrome uses the plain palette tint.
    static let none = GradientTheme(dots: [], opacity: 0.5, texture: 0, schemeOverride: .auto)

    var isEmpty: Bool { dots.isEmpty }

    // MARK: Solid colors

    /// Marker prefix on `presetID` identifying a single-color "solid" theme, so
    /// the pickers can recognise and highlight it without comparing dot UUIDs.
    static let solidPrefix = "solid:"

    /// A flat, single-color chrome theme. One dot reads as a solid tint through
    /// `GradientMesh`; the scheme stays `.auto` so the engine infers light/dark
    /// (and thus legible chrome text) from the color's own luminance.
    static func solid(_ rgb: RGB) -> GradientTheme {
        let (pos, light) = GradientEngine.positionFromColor(rgb)
        let dot = GradientDot(rgb: rgb, x: Double(pos.x), y: Double(pos.y),
                              lightness: light, algorithm: .floating,
                              isPrimary: true, isCustom: true)
        return GradientTheme(dots: [dot], opacity: 0.6, texture: 0,
                             schemeOverride: .auto,
                             presetID: solidPrefix + rgb.token.hexString)
    }

    /// This theme's color when it is a solid theme (`#rrggbb`), else `nil`.
    var solidHex: String? {
        guard let presetID, presetID.hasPrefix(Self.solidPrefix) else { return nil }
        return String(presetID.dropFirst(Self.solidPrefix.count))
    }

    var isSolid: Bool { solidHex != nil }
}
