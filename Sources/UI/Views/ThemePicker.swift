import SwiftUI

/// The theme gallery: a grid of ready-made anime presets plus a "Default" tile
/// that clears the theme. Selecting a tile writes its `GradientTheme` to
/// `BrowserSettings.shared.gradientTheme`, so the chrome wash and derived accent
/// update immediately. (The earlier freeform color picker was retired in favor
/// of curated presets.)
struct ThemePicker: View {
    @ObservedObject private var settings = BrowserSettings.shared
    @Environment(\.palette) private var p

    private let columns = [GridItem(.flexible(), spacing: 12),
                           GridItem(.flexible(), spacing: 12)]

    private var activePresetID: String? { settings.gradientTheme.presetID }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            LazyVGrid(columns: columns, spacing: 12) {
                DefaultTile(isSelected: settings.gradientTheme.isEmpty) {
                    settings.gradientTheme = .none
                }
                ForEach(ThemePreset.all) { preset in
                    PresetTile(preset: preset,
                               isSelected: activePresetID == preset.id) {
                        settings.gradientTheme = preset.theme
                    }
                }
            }

            VStack(alignment: .leading, spacing: 8) {
                Text("Solid colors")
                    .font(Typography.ui(Typography.label, weight: .medium))
                    .foregroundStyle(p.mutedForeground.color)
                SolidThemeSwatches()
            }
        }
        .frame(width: 320)
    }
}

// MARK: - Solid colors

/// A flow of solid-color chips plus a native color well for any custom color.
/// Selecting a chip washes the chrome in a flat single-color theme; the active
/// chip (or the well, for a hand-picked color) shows a selection ring. Shared by
/// the sidebar popover and the Settings panel so both stay in sync.
struct SolidThemeSwatches: View {
    @ObservedObject private var settings = BrowserSettings.shared

    private var activeHex: String? { settings.gradientTheme.solidHex }

    private let columns = [GridItem(.adaptive(minimum: 30), spacing: 10, alignment: .leading)]

    private func isActive(_ hex: String) -> Bool {
        activeHex?.caseInsensitiveCompare(hex) == .orderedSame
    }

    var body: some View {
        LazyVGrid(columns: columns, alignment: .leading, spacing: 10) {
            ForEach(SolidPalette.swatches, id: \.self) { hex in
                SolidSwatch(hex: hex, isSelected: isActive(hex)) {
                    settings.gradientTheme = .solid(RGB(TokenColor(hex: hex)))
                }
            }
            CustomSolidSwatch()
        }
    }
}

/// One round solid-color chip with a check + ring when it's the active theme.
private struct SolidSwatch: View {
    let hex: String
    let isSelected: Bool
    let action: () -> Void

    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            Circle()
                .fill(TokenColor(hex: hex).color)
                .frame(width: 28, height: 28)
                .overlay(Circle().strokeBorder(.white.opacity(0.18), lineWidth: 1))
                .overlay {
                    if isSelected {
                        Image(systemName: "checkmark")
                            .font(.system(size: 12, weight: .bold))
                            .foregroundStyle(GradientEngine.contrastingText(on: RGB(TokenColor(hex: hex))).color)
                    }
                }
                .overlay {
                    Circle()
                        .strokeBorder(TokenColor(hex: hex).color, lineWidth: 2)
                        .padding(-3)
                        .opacity(isSelected ? 1 : 0)
                }
                .scaleEffect(hovering ? 1.08 : 1)
                .contentShape(Circle())
        }
        .buttonStyle(.plain)
        .help(hex)
        .onHover { hovering = $0 }
        .animation(Motion.state, value: isSelected)
        .animation(Motion.snappy, value: hovering)
    }
}

/// A native color well for an arbitrary solid color. Reflects the current solid
/// theme so reopening shows the picked color, and writes a fresh solid theme on
/// change.
private struct CustomSolidSwatch: View {
    @ObservedObject private var settings = BrowserSettings.shared

    private var binding: Binding<Color> {
        Binding(
            get: {
                settings.gradientTheme.solidHex.map { TokenColor(hex: $0).color } ?? .gray
            },
            set: { settings.gradientTheme = .solid(RGB($0)) }
        )
    }

    var body: some View {
        ColorPicker("", selection: binding, supportsOpacity: false)
            .labelsHidden()
            .frame(width: 28, height: 28)
            .help("Custom color")
    }
}

// MARK: - Settings list

/// The Settings counterpart to `ThemePicker`. Where the sidebar popover shows a
/// compact two-column tile gallery, Settings has room to breathe — so themes are
/// laid out as full-width rows inside a single grouped card: a wide swatch, the
/// name and tagline, and a trailing check on the active row. This reads as a
/// settings list (like the rest of the panel) rather than a gallery.
struct ThemeList: View {
    @ObservedObject private var settings = BrowserSettings.shared
    @Environment(\.palette) private var p

    private var activePresetID: String? { settings.gradientTheme.presetID }

    var body: some View {
        VStack(spacing: 0) {
            ThemeRow(swatch: .neutral,
                     name: "Default",
                     subtitle: "System chrome",
                     isSelected: settings.gradientTheme.isEmpty) {
                settings.gradientTheme = .none
            }
            ForEach(ThemePreset.all) { preset in
                Hairline().opacity(0.5)
                ThemeRow(swatch: .gradient(preset.theme.dots.map(\.rgb.color)),
                         name: preset.name,
                         subtitle: preset.subtitle,
                         isSelected: activePresetID == preset.id) {
                    settings.gradientTheme = preset.theme
                }
            }
        }
        .background(
            RoundedRectangle(cornerRadius: Radius.xl, style: .continuous)
                .fill(p.card.color.opacity(0.4))
        )
        .overlay(
            RoundedRectangle(cornerRadius: Radius.xl, style: .continuous)
                .strokeBorder(p.border.color.opacity(0.6), lineWidth: 1)
        )
        .clipShape(RoundedRectangle(cornerRadius: Radius.xl, style: .continuous))
    }
}

/// A single themed row: wide swatch + name/tagline + a trailing check.
private struct ThemeRow: View {
    enum Swatch {
        case neutral
        case gradient([Color])
    }

    let swatch: Swatch
    let name: String
    let subtitle: String
    let isSelected: Bool
    let action: () -> Void

    @Environment(\.palette) private var p
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            HStack(spacing: 12) {
                swatchView
                    .frame(width: 60, height: 36)
                    .clipShape(RoundedRectangle(cornerRadius: Radius.md + 2, style: .continuous))
                    .overlay(
                        RoundedRectangle(cornerRadius: Radius.md + 2, style: .continuous)
                            .strokeBorder(.white.opacity(0.12), lineWidth: 1)
                    )

                VStack(alignment: .leading, spacing: 1) {
                    Text(name)
                        .font(Typography.ui(Typography.base, weight: .medium))
                        .foregroundStyle(p.foreground.color)
                    Text(subtitle)
                        .font(Typography.ui(Typography.label))
                        .foregroundStyle(p.mutedForeground.color)
                }

                Spacer(minLength: 0)

                Image(systemName: "checkmark.circle.fill")
                    .font(.system(size: 16, weight: .semibold))
                    .foregroundStyle(p.primary.color)
                    .opacity(isSelected ? 1 : 0)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 9)
            .background(rowBackground)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .animation(Motion.state, value: isSelected)
    }

    @ViewBuilder private var swatchView: some View {
        switch swatch {
        case .neutral:
            LinearGradient(colors: [p.card.color, p.sidebar.color],
                           startPoint: .topLeading, endPoint: .bottomTrailing)
        case .gradient(let colors):
            GradientMesh(colors: colors, relativeBlur: 0.35, maxBlur: 18)
        }
    }

    @ViewBuilder private var rowBackground: some View {
        if isSelected {
            p.primary.color.opacity(0.10)
        } else if hovering {
            p.foreground.color.opacity(0.05)
        }
    }
}

// MARK: - Tiles

/// Shared tile geometry/chrome so the preset and default tiles read as one set.
private enum Tile {
    static let radius: CGFloat = 12
    static let height: CGFloat = 70

    static var shape: RoundedRectangle {
        RoundedRectangle(cornerRadius: radius, style: .continuous)
    }

    /// A faint inner bevel that separates the card from the surface on any
    /// background color (works over gradients and the neutral default alike).
    static var hairline: some View {
        shape.strokeBorder(.white.opacity(0.10), lineWidth: 1)
    }
}

/// A single preset tile: its gradient preview with the name overlaid and a ring
/// when active.
private struct PresetTile: View {
    let preset: ThemePreset
    let isSelected: Bool
    let action: () -> Void

    @Environment(\.palette) private var p
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            ZStack(alignment: .bottomLeading) {
                GradientMesh(colors: preset.theme.dots.map(\.rgb.color))
                // A crisp bottom scrim keeps the label legible without a muddy
                // blurred text shadow.
                LinearGradient(colors: [.clear, .black.opacity(0.45)],
                               startPoint: .center, endPoint: .bottom)
                label
            }
            .frame(height: Tile.height)
            .clipShape(Tile.shape)
            .overlay(Tile.hairline)
            .shadow(color: .black.opacity(hovering ? 0.30 : 0.20),
                    radius: hovering ? 9 : 5, y: hovering ? 4 : 2)
            .overlay(SelectionRing(isSelected: isSelected, accent: p.primary.color))
            .scaleEffect(hovering ? 1.02 : 1)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .animation(Motion.state, value: isSelected)
    }

    private var label: some View {
        VStack(alignment: .leading, spacing: 1) {
            Text(preset.name)
                .font(Typography.ui(Typography.label, weight: .semibold))
            Text(preset.subtitle)
                .font(Typography.ui(Typography.small))
                .opacity(0.85)
        }
        .foregroundStyle(.white)
        .shadow(color: .black.opacity(0.35), radius: 1, y: 0.5)
        .padding(10)
    }
}

/// The "no theme" tile — restores the plain light/dark chrome. Styled to match
/// the preset tiles (two-line label, same chrome) with a quiet neutral wash so
/// it reads as a deliberate option rather than an empty slot.
private struct DefaultTile: View {
    let isSelected: Bool
    let action: () -> Void

    @Environment(\.palette) private var p
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            ZStack(alignment: .bottomLeading) {
                LinearGradient(colors: [p.card.color, p.sidebar.color],
                               startPoint: .topLeading, endPoint: .bottomTrailing)
                VStack(alignment: .leading, spacing: 1) {
                    Text("Default")
                        .font(Typography.ui(Typography.label, weight: .semibold))
                    Text("System chrome")
                        .font(Typography.ui(Typography.small))
                        .opacity(0.7)
                }
                .foregroundStyle(p.foreground.color)
                .padding(10)
            }
            .frame(height: Tile.height)
            .clipShape(Tile.shape)
            .overlay(Tile.hairline)
            .shadow(color: .black.opacity(hovering ? 0.30 : 0.20),
                    radius: hovering ? 9 : 5, y: hovering ? 4 : 2)
            .overlay(SelectionRing(isSelected: isSelected, accent: p.primary.color))
            .scaleEffect(hovering ? 1.02 : 1)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .animation(Motion.state, value: isSelected)
    }
}

/// A single accent ring drawn just outside the card so a gap of the surface
/// shows through — clean separation on any tile color, no doubled outline.
private struct SelectionRing: View {
    let isSelected: Bool
    let accent: Color

    var body: some View {
        if isSelected {
            RoundedRectangle(cornerRadius: Tile.radius + 3, style: .continuous)
                .strokeBorder(accent, lineWidth: 2)
                .padding(-4)
                .shadow(color: accent.opacity(0.6), radius: 5)
        }
    }
}
