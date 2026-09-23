import Foundation

/// Claude Code: `~/.claude/projects/<project>/<session>.jsonl`, one line per content block.
final class ClaudeCodeProvider: UsageProvider {
    let watchPaths: [URL] = {
        var roots = [
            Paths.home.appendingPathComponent(".claude/projects"),
            Paths.home.appendingPathComponent(".config/claude/projects"),
        ]
        if let custom = Paths.env("CLAUDE_CONFIG_DIR") {
            roots.insert(custom.appendingPathComponent("projects"), at: 0)
        }
        return roots
    }()

    private let tailer = JSONLTailer()
    private var seen = Set<String>()

    func scan(cutoff: Date, into output: inout ScanOutput) {
        tailer.scan(roots: watchPaths, cutoff: cutoff) { url, line in
            let project = url.deletingLastPathComponent().lastPathComponent
            guard let event = Self.parse(line, fallbackProject: project),
                  event.date >= cutoff,
                  seen.insert(event.key).inserted
            else { return }
            output.events.append(event)
        }
    }

    private struct Line: Decodable {
        let timestamp: String?
        let requestId: String?
        let uuid: String?
        let cwd: String?
        let message: Message?
    }

    private struct Message: Decodable {
        let id: String?
        let model: String?
        let usage: Usage?
    }

    private struct Usage: Decodable {
        let input_tokens: Int?
        let output_tokens: Int?
        let cache_creation_input_tokens: Int?
        let cache_read_input_tokens: Int?
    }

    private static let usageMarker = Data("\"usage\"".utf8)

    static func parse(_ line: Data, fallbackProject: String) -> UsageEvent? {
        // Cheap prefilter: most lines (user turns, tool results) have no usage block.
        guard line.range(of: usageMarker) != nil,
              let entry = try? JSONDecoder().decode(Line.self, from: line),
              let message = entry.message,
              let usage = message.usage,
              let model = message.model, model != "<synthetic>",
              let date = Timestamp.parse(entry.timestamp)
        else { return nil }

        // Every content block of one response shares message id + request id.
        let key: String
        if let id = message.id, let request = entry.requestId {
            key = id + ":" + request
        } else {
            key = message.id ?? entry.uuid ?? UUID().uuidString
        }

        let project = entry.cwd.map { URL(fileURLWithPath: $0).lastPathComponent } ?? ""

        return UsageEvent(
            key: key,
            date: date,
            tool: .claudeCode,
            model: model,
            project: project.isEmpty ? fallbackProject : project,
            input: usage.input_tokens ?? 0,
            output: usage.output_tokens ?? 0,
            cacheWrite: usage.cache_creation_input_tokens ?? 0,
            cacheRead: usage.cache_read_input_tokens ?? 0
        )
    }
}
