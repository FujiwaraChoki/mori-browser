import SwiftUI

/// A named, ready-made gradient theme. Presets are the only way to theme the
/// chrome for now — each is a curated `GradientTheme` evoking an anime's
/// signature palette. The chrome wash and derived UI accent come straight from
/// the existing gradient engine.
struct ThemePreset: Identifiable {
    let id: String
    let name: String
    /// A short evocative tagline shown under the name.
    let subtitle: String
    let theme: GradientTheme

    /// Build a preset from signature hex colors. The first color is the primary
    /// (it drives the derived UI accent); positions are placed on the wheel from
    /// the color so the engine stays consistent.
    private static func make(id: String,
                             name: String,
                             subtitle: String,
                             colors: [String],
                             opacity: Double,
                             texture: Double,
                             scheme: GradientTheme.SchemeMode) -> ThemePreset {
        let dots = colors.enumerated().map { index, hex -> GradientDot in
            let rgb = RGB(TokenColor(hex: hex))
            let (pos, light) = GradientEngine.positionFromColor(rgb)
            return GradientDot(rgb: rgb, x: Double(pos.x), y: Double(pos.y),
                               lightness: light, algorithm: .floating,
                               isPrimary: index == 0, isCustom: true)
        }
        let theme = GradientTheme(dots: dots, opacity: opacity, texture: texture,
                                  schemeOverride: scheme, presetID: id)
        return ThemePreset(id: id, name: name, subtitle: subtitle, theme: theme)
    }

    /// The curated lineup. Evangelion and Tokyo Ghoul were requested; the rest
    /// are recommendations spanning dark, twilight, and a lighter pastel.
    // Palettes drawn from published character/brand color references (color-hex,
    // brandpalettes, schemecolor) and tuned for a cohesive chrome wash. The first
    // color is always the signature accent.
    static let all: [ThemePreset] = [
        // EVA-01: violet body + lime armor + NERV orange. (color-hex #8250/#37729)
        make(id: "evangelion", name: "Evangelion", subtitle: "Unit-01",
             colors: ["#765898", "#52D053", "#E6770B"],
             opacity: 0.58, texture: 0.35, scheme: .dark),

        // Kaneki: blood red → maroon → near-black. (color-hex Kaneki Ken #17748)
        make(id: "tokyo-ghoul", name: "Tokyo Ghoul", subtitle: "Kakugan",
             colors: ["#D11A1F", "#4A0A12", "#0F0A0B"],
             opacity: 0.62, texture: 0.55, scheme: .dark),

        // Tanjiro: teal-green checkered haori over its black. (color-hex Tanjiro)
        make(id: "demon-slayer", name: "Demon Slayer", subtitle: "Checkered Haori",
             colors: ["#4EB18D", "#211F20", "#58C29E"],
             opacity: 0.55, texture: 0.3, scheme: .dark),

        // Gojo: cursed cyan-blue, deep navy, a violet of Sukuna. (schemecolor Gojo)
        make(id: "jujutsu-kaisen", name: "Jujutsu Kaisen", subtitle: "Cursed Energy",
             colors: ["#2BA3E8", "#14182E", "#6A3FA0"],
             opacity: 0.56, texture: 0.32, scheme: .dark),

        // CSM: blood red, grime-black, Pochita orange. (color-hex Denji #1056904)
        make(id: "chainsaw-man", name: "Chainsaw Man", subtitle: "Pochita",
             colors: ["#B52C2F", "#1B1513", "#E07B3A"],
             opacity: 0.6, texture: 0.55, scheme: .dark),

        // Kataware-doki: warm gold horizon, dusk rose, deep indigo sky.
        make(id: "your-name", name: "Your Name", subtitle: "Kataware-doki",
             colors: ["#F4A65B", "#C95B7E", "#2A2D5E"],
             opacity: 0.5, texture: 0.22, scheme: .dark),

        // Magical-girl: crystal pink, moonlight blue, tiara gold (light chrome).
        make(id: "sailor-moon", name: "Sailor Moon", subtitle: "Moonlight",
             colors: ["#FB87B0", "#5C79CE", "#FBD15B"],
             opacity: 0.46, texture: 0.15, scheme: .light),
    ]
}

/// Curated flat colors for the solid-theme picker. Each becomes a single-color
/// chrome wash via `GradientTheme.solid(_:)`; the set spans warm/cool/neutral so
/// there's a sensible default for most tastes without opening the color well.
enum SolidPalette {
    static let swatches: [String] = [
        "#6E56CF", // violet
        "#3B82F6", // blue
        "#0EA5A4", // teal
        "#22C55E", // green
        "#EAB308", // amber
        "#F97316", // orange
        "#EF4444", // red
        "#EC4899", // pink
        "#64748B", // slate
        "#1F2937", // graphite
    ]
}
