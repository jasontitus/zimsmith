import Foundation

let temporary = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
defer { try? FileManager.default.removeItem(at: temporary) }
let inbox = SharedURLInbox(directory: temporary)
let id = try inbox.save(text: "https://example.com/docs/", title: "Docs")
let reopened = SharedURLInbox(directory: temporary)
let restored = try reopened.list()
assert(restored.first?["id"] as? String == id)
do { try reopened.dismiss(id: "../../outside"); assertionFailure("Unsafe ID accepted") } catch {}
try reopened.dismiss(id: id)
try reopened.dismiss(id: id)
let empty = try reopened.list()
assert(empty.isEmpty)
for _ in 0..<50 { try inbox.save(text: "https://example.com/", title: "Link") }
do { try inbox.save(text: "https://example.com/", title: "Full"); assertionFailure("Full inbox accepted another link") } catch {}
for file in try inbox.files() { try FileManager.default.removeItem(at: file) }
let broken = UUID().uuidString.lowercased()
try Data("incomplete".utf8).write(to: temporary.appendingPathComponent(broken + ".json"))
let recovered = try inbox.list()
assert(recovered.count == 1 && recovered[0]["id"] as? String == broken)
try inbox.dismiss(id: broken)
let final = try inbox.list()
assert(final.isEmpty)
print("Shared inbox: persistent links, acknowledgement, bounds, invalid IDs and damaged-file recovery passed.")
