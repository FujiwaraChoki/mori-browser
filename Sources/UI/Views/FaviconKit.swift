import SwiftUI

/// Favicon source selection for Mori: curated brand glyphs for known sites and
/// a domain-tinted monogram fallback instead of a generic globe.
enum FaviconSource {
    /// A bundled, curated brand asset (e.g. `brand-github`).
    case brand(String)
    /// A remote favicon image to load.
    case remote(URL)
    /// No usable image — show a colored monogram derived from the domain.
    case monogram(letter: String, color: Color)

    /// Resolve the best source for a page: curated brand → remote favicon →
    /// monogram. `icon` is the favicon image URL, `page` the site URL.
    static func resolve(icon: String?, page: String?) -> FaviconSource {
        let host = Self.host(from: page)
        if let host, let asset = SiteBrand.asset(forHost: host) {
            return .brand(asset)
        }
        if let icon, let url = URL(string: icon) {
            return .remote(url)
        }
        // No declared icon yet (e.g. a restored or background tab whose browser
        // hasn't been realized): try the site's conventional /favicon.ico so the
        // real icon still shows. AsyncImage falls back to the monogram if it 404s.
        if let host, let url = URL(string: "https://\(host)/favicon.ico") {
            return .remote(url)
        }
        return monogram(for: host)
    }

    /// The monogram to show when an image is missing or fails to load.
    static func monogram(for host: String?) -> FaviconSource {
        .monogram(letter: Self.letter(for: host), color: Self.color(for: host))
    }

    // MARK: Host helpers

    /// Lowercased host without a leading `www.`; tolerates scheme-less input.
    static func host(from page: String?) -> String? {
        guard let page, !page.isEmpty else { return nil }
        let raw = URL(string: page)?.host
            ?? URL(string: "https://\(page)")?.host
        guard var h = raw?.lowercased() else { return nil }
        if h.hasPrefix("www.") { h.removeFirst(4) }
        return h.isEmpty ? nil : h
    }

    private static func letter(for host: String?) -> String {
        guard let host, let first = host.first(where: { $0.isLetter || $0.isNumber }) else {
            return "?"
        }
        return String(first).uppercased()
    }

    /// Deterministic color from the host. A stable djb2 hash (Swift's `Hasher`
    /// is per-run randomized) maps to a hue at fixed saturation/brightness so a
    /// site always gets the same tile color.
    private static func color(for host: String?) -> Color {
        guard let host else { return Color(hue: 0.6, saturation: 0.10, brightness: 0.55) }
        var hash: UInt64 = 5381
        for byte in host.utf8 { hash = (hash &* 33) &+ UInt64(byte) }
        let hue = Double(hash % 360) / 360.0
        return Color(hue: hue, saturation: 0.62, brightness: 0.80)
    }
}

/// Domain → curated brand asset.
enum SiteBrand {
    static let map: [String: String] = [
        "github.com": "brand-github",
        "discord.com": "brand-discord",
        "discord.gg": "brand-discord",
        "notion.so": "brand-notion",
        "notion.com": "brand-notion",
        "slack.com": "brand-slack",
        "figma.com": "brand-figma",
        "trello.com": "brand-trello",
        "obsidian.md": "brand-obsidian",
        "tuta.com": "brand-tuta",
        "tutanota.com": "brand-tuta",
        "calendar.google.com": "brand-calendar",
    ]

    /// Matches the host exactly or as a subdomain of a mapped registrable domain
    /// (so `gist.github.com` still resolves to GitHub).
    static func asset(forHost host: String) -> String? {
        if let exact = map[host] { return exact }
        for (domain, asset) in map where host.hasSuffix("." + domain) {
            return asset
        }
        return nil
    }
}

/// A colored, rounded monogram tile shown when a site has no usable favicon.
struct FaviconMonogram: View {
    let letter: String
    let color: Color
    var size: CGFloat

    var body: some View {
        RoundedRectangle(cornerRadius: size * 0.27, style: .continuous)
            .fill(
                LinearGradient(
                    colors: [color, color.opacity(0.78)],
                    startPoint: .top, endPoint: .bottom
                )
            )
            .overlay(
                Text(letter)
                    .font(.system(size: size * 0.6, weight: .semibold, design: .rounded))
                    .foregroundStyle(.white)
            )
            .frame(width: size, height: size)
    }
}
