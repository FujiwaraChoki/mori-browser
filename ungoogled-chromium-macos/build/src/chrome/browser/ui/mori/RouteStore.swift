import SwiftUI

/// An Air Traffic Control rule: pages on `pattern` (a host or host suffix) are
/// routed into the space identified by `contextID`.
struct RoutingRule: Codable, Identifiable {
    var id = UUID()
    var pattern: String
    var contextID: UUID
    var enabled: Bool = true
}

/// Persistent routing rules, JSON-backed in Application Support.
final class RouteStore: ObservableObject {
    static let shared = RouteStore()

    @Published private(set) var rules: [RoutingRule] = []

    private let fileURL: URL
    private var saveScheduled = false

    init() {
        let fm = FileManager.default
        let base = fm.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
            ?? fm.temporaryDirectory
        let dir = base.appendingPathComponent("MoriBrowser", isDirectory: true)
        try? fm.createDirectory(at: dir, withIntermediateDirectories: true)
        fileURL = dir.appendingPathComponent("routes.json")
        load()
    }

    /// The most specific (longest-pattern) enabled rule matching this URL.
    func matchingContextID(forURL url: String) -> UUID? {
        guard let host = URLComponents(string: url)?.host?.lowercased() else { return nil }
        return rules
            .filter { $0.enabled && (host == $0.pattern || host.hasSuffix("." + $0.pattern)) }
            .max(by: { $0.pattern.count < $1.pattern.count })?
            .contextID
    }

    /// Normalize free-form input into a bare host pattern (no scheme/path/www).
    static func normalize(_ raw: String) -> String {
        var text = raw.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        if let host = URLComponents(string: text)?.host { text = host }
        if let slash = text.firstIndex(of: "/") { text = String(text[..<slash]) }
        if let colon = text.firstIndex(of: ":") { text = String(text[..<colon]) }
        if text.hasPrefix("www.") { text = String(text.dropFirst(4)) }
        return text
    }

    @discardableResult
    func add(pattern raw: String, contextID: UUID) -> Bool {
        let pattern = Self.normalize(raw)
        guard !pattern.isEmpty, pattern.contains(".") else { return false }
        rules.removeAll { $0.pattern == pattern }
        rules.insert(RoutingRule(pattern: pattern, contextID: contextID), at: 0)
        scheduleSave()
        return true
    }

    func remove(_ rule: RoutingRule) {
        rules.removeAll { $0.id == rule.id }
        scheduleSave()
    }

    func setEnabled(_ enabled: Bool, for rule: RoutingRule) {
        guard let idx = rules.firstIndex(where: { $0.id == rule.id }) else { return }
        rules[idx].enabled = enabled
        scheduleSave()
    }

    /// Drop rules pointing at a context that's being deleted.
    func removeRules(forContext id: UUID) {
        let before = rules.count
        rules.removeAll { $0.contextID == id }
        if rules.count != before { scheduleSave() }
    }

    // MARK: Persistence

    private func load() {
        guard let data = try? Data(contentsOf: fileURL),
              let decoded = try? JSONDecoder().decode([RoutingRule].self, from: data)
        else { return }
        rules = decoded
    }

    private func scheduleSave() {
        guard !saveScheduled else { return }
        saveScheduled = true
        DispatchQueue.main.async { [weak self] in
            self?.saveScheduled = false
            self?.save()
        }
    }

    private func save() {
        guard let data = try? JSONEncoder().encode(rules) else { return }
        try? data.write(to: fileURL, options: .atomic)
    }
}
