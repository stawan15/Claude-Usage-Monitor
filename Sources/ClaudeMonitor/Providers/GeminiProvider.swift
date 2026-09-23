import Foundation

/// Gemini CLI: session recordings under `~/.gemini/tmp/<project>/chats/`.
///
/// Current versions append message records to `session-*.jsonl`; older ones rewrite a whole
/// `session-*.json` (`{"messages": [...]}`). Model replies have `type: "gemini"` and `tokens`.
final class GeminiProvider: UsageProvider {
    private let root = (Paths.env("GEMINI_CLI_HOME") ?? Paths.home).appendingPathComponent(".gemini/tmp")

    var watchPaths: [URL] { [root] }

    private let tailer = JSONLTailer()
    private var seen = Set<String>()
    private var jsonStamps: [String: Date] = [:]

    private struct Message: Decodable {
        let id: String?
        let timestamp: String?
        let type: String?
        let model: String?
        let tokens: Tokens?
    }

    private struct Tokens: Decodable {
        let input: Int?
        let output: Int?
        let cached: Int?
        let thoughts: Int?
        let tool: Int?
    }

    private struct Conversation: Decodable {
        let messages: [Message]?
    }

    private static func isChatFile(_ url: URL, ext: String) -> Bool {
        url.pathExtension == ext && url.pathComponents.contains("chats")
    }

    func scan(cutoff: Date, into output: inout ScanOutput) {
        tailer.scan(roots: [root], cutoff: cutoff, include: { Self.isChatFile($0, ext: "jsonl") }) { url, line in
            guard let message = try? JSONDecoder().decode(Message.self, from: line) else { return }
            record(message, file: url, cutoff: cutoff, into: &output)
        }
        scanLegacyJSON(cutoff: cutoff, into: &output)
    }

    private func scanLegacyJSON(cutoff: Date, into output: inout ScanOutput) {
        let fm = FileManager.default
        guard let files = fm.enumerator(at: root, includingPropertiesForKeys: [.contentModificationDateKey]) else { return }
        for case let url as URL in files where Self.isChatFile(url, ext: "json") {
            guard let modified = (try? url.resourceValues(forKeys: [.contentModificationDateKey]))?.contentModificationDate,
                  modified >= cutoff,
                  jsonStamps[url.path] != modified,
                  let data = try? Data(contentsOf: url),
                  let conversation = try? JSONDecoder().decode(Conversation.self, from: data)
            else { continue }
            jsonStamps[url.path] = modified
            for message in conversation.messages ?? [] {
                record(message, file: url, cutoff: cutoff, into: &output)
            }
        }
    }

    private func record(_ message: Message, file: URL, cutoff: Date, into output: inout ScanOutput) {
        guard message.type == "gemini",
              let id = message.id,
              let tokens = message.tokens,
              let date = Timestamp.parse(message.timestamp), date >= cutoff
        else { return }

        let session = file.deletingPathExtension().lastPathComponent
        let key = "gemini:\(session):\(id)"
        guard seen.insert(key).inserted else { return }

        // Gemini's prompt count includes cached tokens; thoughts are billed as output.
        let cached = tokens.cached ?? 0
        let project = file.deletingLastPathComponent().deletingLastPathComponent().lastPathComponent
        output.events.append(UsageEvent(
            key: key,
            date: date,
            tool: .gemini,
            model: message.model ?? "gemini",
            project: project,
            input: max(0, (tokens.input ?? 0) - cached) + (tokens.tool ?? 0),
            output: (tokens.output ?? 0) + (tokens.thoughts ?? 0),
            cacheWrite: 0,
            cacheRead: cached
        ))
    }
}
