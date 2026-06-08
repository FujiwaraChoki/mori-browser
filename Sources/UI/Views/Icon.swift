import SwiftUI

/// Central icon registry. Mori renders its bundled Nucleo assets (used under
/// the project's Nucleo license) as tintable template images. Call sites keep
/// using SF-Symbol-style identifiers so the code reads naturally and
/// dynamically-computed names keep resolving; anything not in `map` falls back
/// to the real SF Symbol so nothing silently vanishes.
///
/// Every name Mori uses is currently mapped. The SF fallback remains a safety
/// net for any new name added before its asset.
enum Nucleo {
    /// SF-style name → (asset name in the catalog, clockwise rotation°).
    /// Directional chevrons reuse one right-pointing asset, rotated.
    static let map: [String: (asset: String, rotation: Double)] = [
        "mori": ("mori", 0),
        "sparkles": ("sparkles", 0),
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
        "music.note": ("audio-mixer", 0),
        "play.rectangle.fill": ("half-dotted-circle-play", 0),
        "pip.enter": ("minimize-window", 0),
        // No dedicated exit glyph — the enter window rotated 180° reads as
        // "expand out of PiP" and pairs as a natural toggle with pip.enter.
        "pip.exit": ("minimize-window", 180),
    ]
}

/// A single icon sized by `size` (the point box it occupies). Tint with
/// `.foregroundStyle(...)` at the call site, as with SF Symbols.
struct Icon: View {
    let name: String
    var size: CGFloat = 16
    var weight: Font.Weight = .medium

    var body: some View {
        if let spec = Nucleo.map[name] {
            Image(spec.asset)
                .resizable()
                .renderingMode(.template)
                .aspectRatio(contentMode: .fit)
                .frame(width: size, height: size)
                .rotationEffect(.degrees(spec.rotation))
        } else {
            // SF Symbol fallback — match the visual ink of an equivalent asset box.
            Image(systemName: name)
                .font(.system(size: size * 0.82, weight: weight))
        }
    }
}
