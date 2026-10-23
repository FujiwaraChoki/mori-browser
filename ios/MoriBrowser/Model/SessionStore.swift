import Foundation

struct SessionStore: Sendable {
    var directory: URL

    static func live() -> SessionStore {
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        return SessionStore(directory: base.appendingPathComponent("Mori", isDirectory: true))
    }

    func load() -> Workspace {
        let url = fileURL
        guard let data = try? Data(contentsOf: url) else { return .fresh() }
        do {
            return Workspace.restored(try Self.decoder.decode(Snapshot.self, from: data))
        } catch {
            let broken = directory.appendingPathComponent("session.json.broken")
            try? FileManager.default.removeItem(at: broken)
            try? FileManager.default.moveItem(at: url, to: broken)
            return .fresh()
        }
    }

    func save(_ workspace: Workspace) {
        do {
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            let data = try Self.encoder.encode(workspace.snapshot())
            try data.write(to: fileURL, options: .atomic)
        } catch {
            return
        }
    }

    private var fileURL: URL {
        directory.appendingPathComponent("session.json")
    }

    private static let encoder: JSONEncoder = {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        encoder.dateEncodingStrategy = .iso8601
        return encoder
    }()

    private static let decoder: JSONDecoder = {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        return decoder
    }()
}
