import Foundation

/// Shared by the app and its extension. Atomic, per-link files avoid lost writes
/// when the two processes access the App Group simultaneously.
struct SharedURLInbox {
    let directory: URL

    init(directory: URL) { self.directory = directory }

    static func appGroup() throws -> SharedURLInbox {
        let bundle = Bundle.main.bundleIdentifier ?? "org.zimstudio.app"
        let parent = bundle.hasSuffix(".share") ? String(bundle.dropLast(6)) : bundle
        guard let container = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: "group.\(parent)") else {
            throw failure("Shared-link storage is unavailable. The app and extension need the same App Group.")
        }
        return SharedURLInbox(directory: container.appendingPathComponent("shared-links", isDirectory: true))
    }

    static func failure(_ message: String) -> NSError {
        NSError(domain: "ZimsmithSharing", code: 1, userInfo: [NSLocalizedDescriptionKey: message])
    }

    func files() throws -> [URL] {
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        return try FileManager.default.contentsOfDirectory(at: directory, includingPropertiesForKeys: [.fileSizeKey])
            .filter { $0.pathExtension == "json" }.sorted { $0.lastPathComponent < $1.lastPathComponent }
    }

    @discardableResult func save(text: String, title: String) throws -> String {
        guard text.utf8.count <= 32_768 else { throw Self.failure("Share a single website link.") }
        guard try files().count < 50 else { throw Self.failure("Open Zimsmith and use or dismiss your saved links first.") }
        let id = UUID().uuidString.lowercased()
        let item: [String: Any] = ["id": id, "text": text, "title": String(title.prefix(180)), "created_at": Int64(Date().timeIntervalSince1970 * 1000)]
        try JSONSerialization.data(withJSONObject: item).write(to: directory.appendingPathComponent(id + ".json"), options: .atomic)
        return id
    }

    func list() throws -> [[String: Any]] {
        try files().prefix(50).map { url in
            let size = (try? url.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0
            guard size <= 65_536,
                  let data = try? Data(contentsOf: url),
                  let item = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else {
                return ["id": url.deletingPathExtension().lastPathComponent, "text": "", "title": "Unusable shared link"]
            }
            return item
        }.sorted { ($0["created_at"] as? Int64 ?? 0) < ($1["created_at"] as? Int64 ?? 0) }
    }

    func dismiss(id: String) throws {
        guard let uuid = UUID(uuidString: id), uuid.uuidString.lowercased() == id else { throw Self.failure("Invalid shared-link identifier.") }
        let file = directory.appendingPathComponent(id + ".json")
        if FileManager.default.fileExists(atPath: file.path) { try FileManager.default.removeItem(at: file) }
    }
}
