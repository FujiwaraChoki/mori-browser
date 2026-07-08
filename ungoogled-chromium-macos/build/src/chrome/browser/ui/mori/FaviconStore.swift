import AppKit
import Foundation

/// Persistent decoded favicon cache keyed by lowercased http(s) host. Session
/// restore and sleeping tabs can render favicons without realizing a browser.
final class FaviconStore {
    static let shared = FaviconStore()

    private var images: [String: NSImage] = [:]
    private let lock = NSLock()
    private let directoryURL: URL
    private let writeQueue = DispatchQueue(label: "mori.favicon-store.write",
                                           qos: .utility)

    private init() {
        directoryURL = Self.supportDirectory()
            .appendingPathComponent("favicons", isDirectory: true)
        try? FileManager.default.createDirectory(at: directoryURL,
                                                 withIntermediateDirectories: true)
    }

    /// Returns the cached favicon for an http(s) page host. Disk reads are
    /// synchronous so restored tab rows can be populated before their first draw.
    func image(forPage urlString: String?) -> NSImage? {
        guard let host = Self.host(forPage: urlString) else { return nil }
        if let image = cachedImage(forHost: host) { return image }

        let fileURL = directoryURL.appendingPathComponent(Self.filename(forHost: host))
        guard let image = NSImage(contentsOf: fileURL) else { return nil }
        cache(image, forHost: host)
        return image
    }

    /// Stores a decoded favicon for an http(s) page host. Only successful image
    /// decodes reach this API; nil favicon downloads keep their previous image.
    func store(_ image: NSImage, forPage urlString: String?) {
        guard let host = Self.host(forPage: urlString),
              let data = Self.pngData(from: image)
        else { return }

        cache(image, forHost: host)
        let fileURL = directoryURL.appendingPathComponent(Self.filename(forHost: host))
        writeQueue.async {
            try? data.write(to: fileURL, options: .atomic)
        }
    }

    private func cachedImage(forHost host: String) -> NSImage? {
        lock.lock()
        defer { lock.unlock() }
        return images[host]
    }

    private func cache(_ image: NSImage, forHost host: String) {
        lock.lock()
        images[host] = image
        lock.unlock()
    }

    private static func supportDirectory() -> URL {
        let fm = FileManager.default
        let base = fm.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
            ?? fm.temporaryDirectory
        let dir = base.appendingPathComponent("MoriBrowser", isDirectory: true)
        try? fm.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    /// Favicon identity is the page's http(s) host, normalized like
    /// `SiteBrand.host` (lowercased, `www.`-stripped) so cache keys and
    /// same-site checks agree with brand-glyph resolution.
    static func host(forPage urlString: String?) -> String? {
        guard let raw = urlString?.trimmingCharacters(in: .whitespacesAndNewlines),
              let components = URLComponents(string: raw),
              let scheme = components.scheme?.lowercased(),
              scheme == "http" || scheme == "https",
              var host = components.host?.lowercased(),
              !host.isEmpty
        else { return nil }
        if host.hasPrefix("www.") { host.removeFirst(4) }
        return host.isEmpty ? nil : host
    }

    private static func filename(forHost host: String) -> String {
        let allowed = CharacterSet(charactersIn: "abcdefghijklmnopqrstuvwxyz0123456789.-_")
        var sanitized = ""
        for scalar in host.unicodeScalars {
            sanitized.append(allowed.contains(scalar) ? Character(scalar) : "-")
        }
        if sanitized.isEmpty { sanitized = "host" }
        return "\(sanitized)-\(stableHash(host)).png"
    }

    private static func stableHash(_ value: String) -> String {
        var hash: UInt64 = 14_695_981_039_346_656_037
        for byte in value.utf8 {
            hash ^= UInt64(byte)
            hash = hash &* 1_099_511_628_211
        }
        return String(format: "%016llx", hash)
    }

    private static func pngData(from image: NSImage) -> Data? {
        if let cgImage = image.cgImage(forProposedRect: nil, context: nil, hints: nil) {
            let rep = NSBitmapImageRep(cgImage: cgImage)
            return rep.representation(using: .png, properties: [:])
        }
        guard let tiffData = image.tiffRepresentation,
              let rep = NSBitmapImageRep(data: tiffData)
        else { return nil }
        return rep.representation(using: .png, properties: [:])
    }
}
