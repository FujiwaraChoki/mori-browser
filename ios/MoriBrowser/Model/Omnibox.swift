import Foundation

enum Omnibox {
    enum Resolution: Equatable {
        case url(URL)
        case search(String, URL)
    }

    static func resolve(_ raw: String, engine: SearchEngine) -> Resolution? {
        let query = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !query.isEmpty else { return nil }
        if let url = directURL(query) {
            return .url(url)
        }
        return .search(query, engine.searchURL(for: query))
    }

    private static func directURL(_ query: String) -> URL? {
        if query.contains(where: \.isWhitespace) { return nil }
        if let scheme = explicitScheme(of: query) {
            guard scheme == "http" || scheme == "https", let url = URL(string: query) else { return nil }
            return url
        }
        let pathHost = query.split(separator: "/", maxSplits: 1, omittingEmptySubsequences: false).first.map(String.init) ?? query
        let host = pathHost.split(separator: ":", maxSplits: 1, omittingEmptySubsequences: false).first.map(String.init) ?? pathHost
        if isIPv4(host) || prefersInsecure(host) {
            let scheme = prefersInsecure(host) ? "http" : "https"
            return URL(string: "\(scheme)://\(query)")
        }
        guard looksLikeDomain(query) else { return nil }
        return URL(string: "https://\(query)")
    }

    /// A scheme only counts when it is written as `scheme://`. `localhost:8080` is a port.
    private static func explicitScheme(of query: String) -> String? {
        guard let index = query.firstIndex(of: ":"), index != query.startIndex else { return nil }
        let scheme = query[..<index]
        guard scheme.allSatisfy(\.isLetter) else { return nil }
        let rest = query[query.index(after: index)...]
        guard rest.hasPrefix("//") else { return nil }
        return scheme.lowercased()
    }

    private static func prefersInsecure(_ host: String) -> Bool {
        let name = host.lowercased()
        if name == "localhost" || name.hasSuffix(".local") || name == "::1" { return true }
        guard isIPv4(name) else { return false }
        let parts = name.split(separator: ".").compactMap { Int($0) }
        guard parts.count == 4 else { return true }
        if parts[0] == 10 || parts[0] == 127 { return true }
        if parts[0] == 192 && parts[1] == 168 { return true }
        if parts[0] == 172 && (16...31).contains(parts[1]) { return true }
        return false
    }

    private static func isIPv4(_ host: String) -> Bool {
        let parts = host.split(separator: ".", omittingEmptySubsequences: false)
        guard parts.count == 4 else { return false }
        return parts.allSatisfy { part in
            guard !part.isEmpty, part.count <= 3, let value = Int(part) else { return false }
            return (0...255).contains(value)
        }
    }

    private static func looksLikeDomain(_ query: String) -> Bool {
        let pattern = #"^([a-zA-Z0-9-]+\.)+[A-Za-z]{2,}(:\d{1,5})?(/[^\s]*)?$"#
        return query.range(of: pattern, options: .regularExpression) != nil
    }
}

enum SuggestionParser {
    static func parse(_ data: Data) -> [String] {
        guard let json = try? JSONSerialization.jsonObject(with: data) else { return [] }
        let phrases: [String]
        if let array = json as? [Any], array.count >= 2 {
            phrases = strings(from: array[1])
        } else if let objects = json as? [[String: Any]] {
            phrases = objects.compactMap { $0["phrase"] as? String }
        } else {
            phrases = []
        }
        return uniqueNonEmpty(phrases).prefix(8).map { $0 }
    }

    private static func strings(from value: Any) -> [String] {
        if let strings = value as? [String] { return strings }
        guard let nested = value as? [Any] else { return [] }
        return nested.flatMap { item -> [String] in
            if let string = item as? String { return [string] }
            if let inner = item as? [Any], let first = inner.first as? String { return [first] }
            return []
        }
    }

    private static func uniqueNonEmpty(_ values: [String]) -> [String] {
        var seen = Set<String>()
        return values.filter { value in
            let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !trimmed.isEmpty else { return false }
            return seen.insert(trimmed.lowercased()).inserted
        }
    }
}

enum SuggestionClient {
    static func fetch(_ query: String, engine: SearchEngine) async -> [String] {
        let trimmed = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed.count >= 2, let url = engine.suggestURL(for: trimmed) else { return [] }
        var request = URLRequest(url: url, timeoutInterval: 4)
        request.setValue("Mozilla/5.0", forHTTPHeaderField: "User-Agent")
        do {
            let (data, response) = try await URLSession.shared.data(for: request)
            guard let http = response as? HTTPURLResponse, http.statusCode == 200 else { return [] }
            return SuggestionParser.parse(data)
        } catch {
            return []
        }
    }
}
