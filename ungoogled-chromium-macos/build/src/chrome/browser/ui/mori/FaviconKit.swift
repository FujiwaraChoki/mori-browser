import SwiftUI

/// Favicon resolution for Mori. A curated brand glyph wins for known sites;
/// otherwise the UI shows the Chromium-decoded site favicon (supplied via
/// `BrowserTab.faviconImage`), falling back to a neutral web globe when a page
/// exposes no favicon at all. No host-derived letter tiles.
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

    /// Curated brand asset for a page URL, if one exists for its host.
    static func asset(forPage page: String?) -> String? {
        guard let host = host(from: page) else { return nil }
        return asset(forHost: host)
    }

    /// Matches the host exactly or as a subdomain of a mapped registrable domain
    /// (so `gist.github.com` still resolves to GitHub).
    static func asset(forHost host: String) -> String? {
        if let exact = map[host] { return exact }
        for (domain, asset) in map where host.hasSuffix("." + domain) {
            return asset
        }
        return nil
    }

    /// Lowercased host without a leading `www.`; tolerates scheme-less input.
    static func host(from page: String?) -> String? {
        guard let page, !page.isEmpty else { return nil }
        let raw = URL(string: page)?.host
            ?? URL(string: "https://\(page)")?.host
        guard var h = raw?.lowercased() else { return nil }
        if h.hasPrefix("www.") { h.removeFirst(4) }
        return h.isEmpty ? nil : h
    }
}
