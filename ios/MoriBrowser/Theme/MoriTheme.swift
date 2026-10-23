import SwiftUI
import UIKit

enum MoriColor {
    static func srgb(lightness: Double, chroma: Double, hue: Double) -> (Double, Double, Double) {
        let radians = hue * .pi / 180
        let a = chroma * cos(radians)
        let b = chroma * sin(radians)
        let l = lightness + 0.3963377774 * a + 0.2158037573 * b
        let m = lightness - 0.1055613458 * a - 0.0638541728 * b
        let s = lightness - 0.0894841775 * a - 1.2914855480 * b
        let l3 = l * l * l
        let m3 = m * m * m
        let s3 = s * s * s
        return (
            gamma(4.0767416621 * l3 - 3.3077115913 * m3 + 0.2309699292 * s3),
            gamma(-1.2684380046 * l3 + 2.6097574011 * m3 - 0.3413193965 * s3),
            gamma(-0.0041960863 * l3 - 0.7034186147 * m3 + 1.7076147010 * s3)
        )
    }

    static func gamma(_ linear: Double) -> Double {
        let value = min(max(linear, 0), 1)
        if value <= 0.0031308 { return 12.92 * value }
        return 1.055 * pow(value, 1 / 2.4) - 0.055
    }

    static func luminance(red: Double, green: Double, blue: Double) -> Double {
        func channel(_ value: Double) -> Double {
            value <= 0.04045 ? value / 12.92 : pow((value + 0.055) / 1.055, 2.4)
        }
        return 0.2126 * channel(red) + 0.7152 * channel(green) + 0.0722 * channel(blue)
    }

    static func components(hex: UInt32) -> (Double, Double, Double) {
        (
            Double((hex >> 16) & 0xFF) / 255,
            Double((hex >> 8) & 0xFF) / 255,
            Double(hex & 0xFF) / 255
        )
    }
}

extension Color {
    init(hex: UInt32, opacity: Double = 1) {
        let (red, green, blue) = MoriColor.components(hex: hex)
        self.init(.sRGB, red: red, green: green, blue: blue, opacity: opacity)
    }
}

struct ThemePreset: Identifiable, Equatable {
    let id: String
    let name: String
    let subtitle: String
    let colors: [UInt32]
    let opacity: Double
    let scheme: GradientTheme.ForcedScheme

    var theme: GradientTheme {
        GradientTheme(colors: colors, opacity: opacity, scheme: scheme, presetID: id)
    }

    static let all: [ThemePreset] = [
        ThemePreset(id: "evangelion", name: "Evangelion", subtitle: "Unit-01", colors: [0x765898, 0x52D053, 0xE6770B], opacity: 0.58, scheme: .dark),
        ThemePreset(id: "tokyo-ghoul", name: "Tokyo Ghoul", subtitle: "Kakugan", colors: [0xD11A1F, 0x4A0A12, 0x0F0A0B], opacity: 0.62, scheme: .dark),
        ThemePreset(id: "demon-slayer", name: "Demon Slayer", subtitle: "Checkered Haori", colors: [0x4EB18D, 0x211F20, 0x58C29E], opacity: 0.55, scheme: .dark),
        ThemePreset(id: "jujutsu-kaisen", name: "Jujutsu Kaisen", subtitle: "Cursed Energy", colors: [0x2BA3E8, 0x14182E, 0x6A3FA0], opacity: 0.56, scheme: .dark),
        ThemePreset(id: "chainsaw-man", name: "Chainsaw Man", subtitle: "Pochita", colors: [0xB52C2F, 0x1B1513, 0xE07B3A], opacity: 0.60, scheme: .dark),
        ThemePreset(id: "your-name", name: "Your Name", subtitle: "Kataware-doki", colors: [0xF4A65B, 0xC95B7E, 0x2A2D5E], opacity: 0.50, scheme: .dark),
        ThemePreset(id: "sailor-moon", name: "Sailor Moon", subtitle: "Moonlight", colors: [0xFB87B0, 0x5C79CE, 0xFBD15B], opacity: 0.46, scheme: .light),
        ThemePreset(id: "sunset", name: "Sunset", subtitle: "Warm horizon", colors: [0xF08A5D, 0xB84A62, 0x355C7D], opacity: 0.52, scheme: .dark),
        ThemePreset(id: "forest", name: "Forest", subtitle: "Moss canopy", colors: [0x2F7D5C, 0x173F35, 0xA7C957], opacity: 0.54, scheme: .dark),
        ThemePreset(id: "nordic", name: "Nordic", subtitle: "Fjord light", colors: [0x88C0D0, 0x5E81AC, 0xECEFF4], opacity: 0.48, scheme: .light),
        ThemePreset(id: "ocean", name: "Ocean", subtitle: "Blue current", colors: [0x0077B6, 0x00B4D8, 0x023E8A], opacity: 0.52, scheme: .dark),
        ThemePreset(id: "lavender", name: "Lavender", subtitle: "Quiet bloom", colors: [0xB8A1FF, 0xF1C6E7, 0x7C6CB0], opacity: 0.44, scheme: .light),
    ]
}

struct MoriPalette: Equatable {
    var background: Color
    var foreground: Color
    var mutedForeground: Color
    var sidebar: Color
    var card: Color
    var primary: Color
    var primaryForeground: Color
    var selected: Color
    var tile: Color
    var separator: Color
    var destructive: Color
    var popover: Color

    static func resolve(theme: GradientTheme?, scheme: ColorScheme) -> MoriPalette {
        let dark = scheme == .dark
        let primary = accent(theme: theme, scheme: scheme)
        let onPrimary = contrastingText(on: primary)
        return MoriPalette(
            background: Color(hex: dark ? 0x222222 : 0xF7F7F7),
            foreground: Color(hex: dark ? 0xF1F3F5 : 0x1C1F24),
            mutedForeground: Color(hex: dark ? 0xAEB6BF : 0x5C6370),
            sidebar: Color(hex: dark ? 0x151515 : 0xEBEBEB),
            card: Color(hex: dark ? 0x2C2C2C : 0xFBFBFB),
            primary: primary,
            primaryForeground: onPrimary,
            selected: dark ? Color.white.opacity(0.17) : Color.white.opacity(0.72),
            tile: dark ? Color.white.opacity(0.12) : Color.black.opacity(0.07),
            separator: dark ? Color.white.opacity(0.09) : Color.black.opacity(0.10),
            destructive: Color(hex: 0xD4523E),
            popover: dark ? Color(hex: 0x333333) : Color.white
        )
    }

    private static func accent(theme: GradientTheme?, scheme: ColorScheme) -> Color {
        if let hex = theme?.colors.first {
            let (red, green, blue) = MoriColor.components(hex: hex)
            if MoriColor.luminance(red: red, green: green, blue: blue) < 0.18 {
                return Color(.sRGB, red: red * 0.45 + 0.55, green: green * 0.45 + 0.55, blue: blue * 0.45 + 0.55, opacity: 1)
            }
            return Color(hex: hex)
        }
        let (red, green, blue) = scheme == .dark
            ? MoriColor.srgb(lightness: 0.62, chroma: 0.13, hue: 241.5)
            : MoriColor.srgb(lightness: 0.645, chroma: 0.11, hue: 241.2)
        return Color(.sRGB, red: red, green: green, blue: blue, opacity: 1)
    }

    private static func contrastingText(on color: Color) -> Color {
        let resolved = UIColor(color)
        var red: CGFloat = 0
        var green: CGFloat = 0
        var blue: CGFloat = 0
        var alpha: CGFloat = 0
        resolved.getRed(&red, green: &green, blue: &blue, alpha: &alpha)
        let luminance = MoriColor.luminance(red: Double(red), green: Double(green), blue: Double(blue))
        return luminance > 0.55 ? Color(hex: 0x1C1F24) : .white
    }
}

private struct MoriPaletteKey: EnvironmentKey {
    static let defaultValue = MoriPalette.resolve(theme: nil, scheme: .light)
}

extension EnvironmentValues {
    var moriPalette: MoriPalette {
        get { self[MoriPaletteKey.self] }
        set { self[MoriPaletteKey.self] = newValue }
    }
}

struct SpaceCanvas: View {
    var theme: GradientTheme?
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        let dark = scheme == .dark
        ZStack {
            Color(hex: dark ? 0x1A1A1A : 0xF3F3F1)
            if let theme, let first = theme.colors.first {
                let second = theme.colors.dropFirst().first ?? first
                LinearGradient(
                    colors: [
                        Color(hex: first).opacity(dark ? 0.30 : 0.16),
                        Color(hex: second).opacity(dark ? 0.08 : 0.05),
                    ],
                    startPoint: .topLeading,
                    endPoint: .bottomTrailing
                )
            }
        }
    }
}

struct WashBackground: View {
    var theme: GradientTheme?
    @Environment(\.moriPalette) private var palette

    var body: some View {
        ZStack {
            palette.sidebar
            if let theme, !theme.colors.isEmpty {
                mesh(theme).opacity(theme.opacity)
            }
        }
    }

    private func mesh(_ theme: GradientTheme) -> some View {
        let colors = theme.colors.map { Color(hex: $0) }
        let first = colors[0]
        let second = colors.count > 1 ? colors[1] : first
        let third = colors.count > 2 ? colors[2] : second
        return MeshGradient(
            width: 3,
            height: 3,
            points: [
                SIMD2<Float>(0, 0), SIMD2<Float>(0.5, 0), SIMD2<Float>(1, 0),
                SIMD2<Float>(0, 0.45), SIMD2<Float>(0.55, 0.5), SIMD2<Float>(1, 0.55),
                SIMD2<Float>(0, 1), SIMD2<Float>(0.5, 1), SIMD2<Float>(1, 1),
            ],
            colors: [first, second, third, second, third, first, third, first, second]
        )
    }
}
