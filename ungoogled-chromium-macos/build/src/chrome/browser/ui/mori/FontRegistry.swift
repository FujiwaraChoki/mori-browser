import AppKit

/// Resolves the UI font family. Mori's webapp uses Söhne, but it ships only as
/// `.woff2` (web-only) and is a licensed font, so the native app defaults to the
/// system font (SF Pro) — the correct, license-clean native choice. If a Söhne
/// family happens to be installed system-wide, we honor it for visual parity.
enum FontRegistry {
    static let soehneFamily: String? = {
        let families = NSFontManager.shared.availableFontFamilies
        let candidates = ["Söhne", "Soehne", "Söhne Buch", "Soehne Buch"]
        for name in candidates where families.contains(name) {
            return name
        }
        return nil
    }()
}
