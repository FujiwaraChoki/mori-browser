import SwiftUI

/// A named, ready-made gradient theme. Each is a curated `GradientTheme`; the
/// chrome wash and derived UI accent come straight from the existing gradient
/// engine.
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

    /// The curated lineup. The first group keeps the original fandom palettes;
    /// the second adds neutral, non-IP options for everyday chrome.
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

        // Warm horizon: coral light, berry shadow, and a muted blue dusk.
        make(id: "sunset", name: "Sunset", subtitle: "Warm horizon",
             colors: ["#F08A5D", "#B84A62", "#355C7D"],
             opacity: 0.52, texture: 0.18, scheme: .dark),

        // Deep evergreen with a moss accent.
        make(id: "forest", name: "Forest", subtitle: "Moss canopy",
             colors: ["#2F7D5C", "#173F35", "#A7C957"],
             opacity: 0.54, texture: 0.24, scheme: .dark),

        // Cool fjord blues over a clean snow highlight.
        make(id: "nordic", name: "Nordic", subtitle: "Fjord light",
             colors: ["#88C0D0", "#5E81AC", "#ECEFF4"],
             opacity: 0.48, texture: 0.12, scheme: .light),

        // Clear marine blues with a deeper current underneath.
        make(id: "ocean", name: "Ocean", subtitle: "Blue current",
             colors: ["#0077B6", "#00B4D8", "#023E8A"],
             opacity: 0.52, texture: 0.20, scheme: .dark),

        // Soft lavender balanced with rose and muted plum.
        make(id: "lavender", name: "Lavender", subtitle: "Quiet bloom",
             colors: ["#B8A1FF", "#F1C6E7", "#7C6CB0"],
             opacity: 0.44, texture: 0.12, scheme: .light),
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

    /// Human-readable names for the swatch hexes, shown on hover instead of the
    /// raw hex (parallels the gradient presets, which show their name).
    static let names: [String: String] = [
        "#6E56CF": "Violet",
        "#3B82F6": "Blue",
        "#0EA5A4": "Teal",
        "#22C55E": "Green",
        "#EAB308": "Amber",
        "#F97316": "Orange",
        "#EF4444": "Red",
        "#EC4899": "Pink",
        "#64748B": "Slate",
        "#1F2937": "Graphite",
    ]
}
