import SwiftUI

/// Central icon registry. Mori renders its bundled Nucleo assets (used under
/// the project's Nucleo license) as tintable template images. Call sites keep
/// using SF-Symbol-style identifiers so the code reads naturally and
/// dynamically-computed names keep resolving; anything not in `map` falls back
/// to the real SF Symbol so nothing silently vanishes.
///
/// Most chrome names Mori uses are mapped. The SF fallback remains a safety
/// net for any name added before a matching asset exists.
enum Nucleo {
    /// SF-style name → (asset name in the catalog, clockwise rotation°).
    /// Directional chevrons reuse one right-pointing asset, rotated.
    static let map: [String: (asset: String, rotation: Double)] = [
        "mori": ("mori", 0),
        "sparkles": ("sparkles", 0),
        "wand.and.stars": ("glyph-star", 0),
        "xmark": ("close", 0),
        "arrow.up": ("arrow-up", 0),
        "arrow.right": ("arrow-right", 0),
        // Line-style nav arrows for the sidebar's back / forward buttons.
        "arrow.backward": ("arrow-line-left", 0),
        "arrow.forward": ("arrow-line-right", 0),
        "arrow.clockwise": ("reload", 0),
        "lock.fill": ("security", 0),
        "exclamationmark.triangle": ("security-warning", 0),
        "magnifyingglass": ("search-glass", 0),
        "magnifier-history": ("magnifier-history", 0),
        "puzzlepiece.extension.fill": ("extension-fill", 0),
        "puzzlepiece.extension": ("extension", 0),
        "clock.arrow.circlepath": ("history", 0),
        "clock": ("history", 0),
        "gearshape": ("settings", 0),
        "pin.fill": ("pin", 0),
        "pin": ("unpin", 0),
        "tray.and.arrow.down": ("downloads", 0),
        "arrow.down.circle": ("downloads", 0),
        "arrow.down.circle.fill": ("downloads", 0),
        "doc.fill": ("page-portrait", 0),
        "sidebar.left": ("sidebar-right", 180),
        "sidebar.right": ("sidebar-right", 0),
        "sidebar.trailing": ("sidebar-right", 0),
        "chevron.right": ("chevron", 0),
        "chevron.left": ("chevron", 180),
        "chevron.down": ("chevron", 90),
        "chevron.up": ("chevron", 270),
        "plus": ("plus", 0),
        "trash": ("trash", 0),
        "sun.max": ("face-sun", 0),
        "moon": ("moon-stars", 0),
        "star": ("bookmark-hollow", 0),
        "star.fill": ("bookmark", 0),
        "paper.plane": ("paper-plane-2", 0),
        "book": ("library", 0),
        "speaker.slash.fill": ("media-mute", 0),
        "speaker.wave.2.fill": ("media-unmute", 0),
        "play.fill": ("media-play", 0),
        "pause.fill": ("media-pause", 0),
        "folder": ("folder", 0),
        "globe": ("earth", 0),
        "wifi.exclamationmark": ("signal-2", 0),
        "chevron.up.chevron.down": ("chevron-down", 0),
        "circle.lefthalf.filled": ("color-palette", 0),
        "slider.horizontal.3": ("glyph-tools", 0),
        "music.note": ("audio-mixer", 0),
        "play.rectangle.fill": ("half-dotted-circle-play", 0),
        "pip.enter": ("minimize-window", 0),
        // No dedicated exit glyph — the enter window rotated 180° reads as
        // "expand out of PiP" and pairs as a natural toggle with pip.enter.
        "pip.exit": ("minimize-window", 180),
    ]
}

/// Customizable glyphs (folder icons, context icons). Unlike the core chrome
/// icons these live as loose Nucleo SVG files under `Resources/MoriGlyphs/`
/// rather than in the compiled asset catalog, so the set can grow without
/// recompiling `Assets.car`. Loaded once as template images (alpha-only) so
/// they tint via `.foregroundStyle(...)` like everything else.
enum GlyphLibrary {
    /// One pickable glyph: the bundled SVG's basename and a human label.
    struct Glyph: Identifiable, Equatable {
        let asset: String
        let label: String
        var id: String { asset }
    }

    /// The picker lineup, in display order (mirrors Arc's folder icon grid).
    static let all: [Glyph] = [
        Glyph(asset: "glyph-star", label: "Star"),
        Glyph(asset: "glyph-bookmark", label: "Bookmark"),
        Glyph(asset: "glyph-heart", label: "Heart"),
        Glyph(asset: "glyph-bolt", label: "Bolt"),
        Glyph(asset: "glyph-triangle", label: "Triangle"),
        Glyph(asset: "glyph-asterisk", label: "Asterisk"),
        Glyph(asset: "glyph-bell", label: "Bell"),
        Glyph(asset: "glyph-lightbulb", label: "Lightbulb"),
        Glyph(asset: "glyph-chat", label: "Chat"),
        Glyph(asset: "glyph-users", label: "People"),
        Glyph(asset: "glyph-tools", label: "Tools"),
        Glyph(asset: "glyph-egg", label: "Egg"),
        Glyph(asset: "glyph-circle", label: "Circle"),
        Glyph(asset: "glyph-moon", label: "Moon"),
        Glyph(asset: "glyph-planet", label: "Planet"),
        Glyph(asset: "glyph-leaf", label: "Leaf"),
        Glyph(asset: "glyph-cloud", label: "Cloud"),
        Glyph(asset: "glyph-paw", label: "Paw"),
        Glyph(asset: "glyph-utensils", label: "Utensils"),
        Glyph(asset: "glyph-plane", label: "Plane"),
        Glyph(asset: "glyph-music", label: "Music"),
        Glyph(asset: "glyph-video", label: "Video"),
        Glyph(asset: "glyph-code", label: "Code"),
        Glyph(asset: "glyph-skull", label: "Skull"),
    ]

    static func isGlyph(_ name: String) -> Bool { name.hasPrefix("glyph-") }

    private static var cache: [String: NSImage] = [:]
    private static let cacheLock = NSLock()

    /// Load a bundled glyph SVG as a tintable template image. Returns nil for
    /// unknown names so callers can fall back.
    static func image(named name: String) -> NSImage? {
        cacheLock.lock()
        defer { cacheLock.unlock() }
        if let cached = cache[name] { return cached }
        guard let base = Bundle.main.resourceURL else { return nil }
        let url = base.appendingPathComponent("MoriGlyphs/\(name).svg")
        guard let image = NSImage(contentsOf: url) else { return nil }
        image.isTemplate = true
        cache[name] = image
        return image
    }
}

/// A single icon sized by `size` (the point box it occupies). Tint with
/// `.foregroundStyle(...)` at the call site, as with SF Symbols.
struct Icon: View {
    let name: String
    var size: CGFloat = 16
    var weight: Font.Weight = .medium

    var body: some View {
        if let spec = Nucleo.map[name] {
            if GlyphLibrary.isGlyph(spec.asset), let glyph = GlyphLibrary.image(named: spec.asset) {
                Image(nsImage: glyph)
                    .resizable()
                    .renderingMode(.template)
                    .aspectRatio(contentMode: .fit)
                    .frame(width: size, height: size)
                    .rotationEffect(.degrees(spec.rotation))
            } else {
                Image(spec.asset)
                    .resizable()
                    .renderingMode(.template)
                    .aspectRatio(contentMode: .fit)
                    .frame(width: size, height: size)
                    .rotationEffect(.degrees(spec.rotation))
            }
        } else if GlyphLibrary.isGlyph(name), let glyph = GlyphLibrary.image(named: name) {
            Image(nsImage: glyph)
                .resizable()
                .renderingMode(.template)
                .aspectRatio(contentMode: .fit)
                .frame(width: size, height: size)
        } else {
            // SF Symbol fallback — match the visual ink of an equivalent asset box.
            Image(systemName: name)
                .font(.system(size: size * 0.82, weight: weight))
        }
    }
}
