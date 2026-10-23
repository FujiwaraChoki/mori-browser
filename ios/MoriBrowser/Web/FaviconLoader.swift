import UIKit

@MainActor
@Observable
final class FaviconLoader {
    static let shared = FaviconLoader()

    private(set) var images: [String: UIImage] = [:]
    private var inflight: Set<String> = []

    func image(for host: String) -> UIImage? {
        images[host.lowercased()]
    }

    func load(host: String) async {
        let key = host.lowercased()
        guard images[key] == nil, inflight.insert(key).inserted else { return }
        defer { inflight.remove(key) }
        if let cached = UIImage(contentsOfFile: fileURL(for: key).path) {
            images[key] = cached
            return
        }
        guard let remote = URL(string: "https://www.google.com/s2/favicons?domain=\(key)&sz=64") else { return }
        do {
            let (data, response) = try await URLSession.shared.data(from: remote)
            guard !Task.isCancelled,
                  let http = response as? HTTPURLResponse,
                  http.statusCode == 200,
                  let image = UIImage(data: data) else { return }
            try? FileManager.default.createDirectory(at: cacheDirectory, withIntermediateDirectories: true)
            try? data.write(to: fileURL(for: key), options: .atomic)
            images[key] = image
        } catch {
            return
        }
    }

    func clear() {
        images = [:]
        inflight = []
        try? FileManager.default.removeItem(at: cacheDirectory)
    }

    private var cacheDirectory: URL {
        FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("MoriFavicons", isDirectory: true)
    }

    private func fileURL(for host: String) -> URL {
        let safe = host.map { character -> Character in
            character.isLetter || character.isNumber || character == "." || character == "-" ? character : "_"
        }
        return cacheDirectory.appendingPathComponent(String(safe) + ".png")
    }
}
