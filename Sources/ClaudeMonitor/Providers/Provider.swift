import Foundation

/// The coding agent a usage record came from.
enum AgentTool: String, CaseIterable, Identifiable, Sendable {
    case claudeCode, codex, openCode, gemini

    var id: Self { self }

    var name: String {
        switch self {
        case .claudeCode: "Claude Code"
        case .codex: "Codex"
        case .openCode: "OpenCode"
        case .gemini: "Gemini CLI"
        }
    }

    var shortName: String {
        switch self {
        case .claudeCode: "Claude"
        case .gemini: "Gemini"
        default: name
        }
    }
}

/// One billed model response.
struct UsageEvent: Sendable {
    let key: String
    let date: Date
    let tool: AgentTool
    let model: String
    let project: String
    /// Uncached input tokens.
    let input: Int
    /// Output tokens, including reasoning/thinking.
    let output: Int
    let cacheWrite: Int
    let cacheRead: Int
    /// Set when the tool reports its own cost (OpenCode); otherwise priced from `Pricing`.
    var cost: Double?

    var total: Int { input + output + cacheWrite + cacheRead }
}

/// A plan rate limit as reported by the tool itself (currently Codex).
struct LimitStatus: Identifiable, Sendable {
    let tool: AgentTool
    let windowMinutes: Int
    let usedPercent: Double
    let resetsAt: Date?
    let observedAt: Date

    var id: String { "\(tool.rawValue)-\(windowMinutes)" }
}

struct ScanOutput: Sendable {
    var events: [UsageEvent] = []
    var limits: [LimitStatus] = []
}

/// Reads one tool's local logs. Called only from `UsageWatcher`'s serial queue.
protocol UsageProvider: AnyObject {
    /// Directories to watch for changes. They need not exist yet.
    var watchPaths: [URL] { get }
    /// Appends anything new since the previous scan.
    func scan(cutoff: Date, into output: inout ScanOutput)
}

enum Paths {
    static let home = FileManager.default.homeDirectoryForCurrentUser

    static func env(_ name: String) -> URL? {
        ProcessInfo.processInfo.environment[name].map { URL(fileURLWithPath: $0) }
    }
}

enum Timestamp {
    private static let fractional = Date.ISO8601FormatStyle(includingFractionalSeconds: true)
    private static let whole = Date.ISO8601FormatStyle()

    static func parse(_ s: String?) -> Date? {
        guard let s else { return nil }
        return (try? fractional.parse(s)) ?? (try? whole.parse(s))
    }
}

/// Tails append-only JSONL files, handing each new complete line to a callback.
final class JSONLTailer {
    private var offsets: [String: UInt64] = [:]

    func scan(roots: [URL], cutoff: Date, include: (URL) -> Bool = { $0.pathExtension == "jsonl" }, onLine: (URL, Data) -> Void) {
        let fm = FileManager.default
        let keys: [URLResourceKey] = [.fileSizeKey, .contentModificationDateKey, .isRegularFileKey]

        for root in roots where fm.fileExists(atPath: root.path) {
            guard let files = fm.enumerator(at: root, includingPropertiesForKeys: keys, options: [.skipsHiddenFiles]) else { continue }

            for case let url as URL in files where include(url) {
                guard let values = try? url.resourceValues(forKeys: Set(keys)),
                      values.isRegularFile == true,
                      let size = values.fileSize.map(UInt64.init)
                else { continue }

                let path = url.path
                var offset = offsets[path] ?? 0
                if size < offset { offset = 0 } // truncated or replaced
                if size == offset { continue }

                // First sighting of a stale file: skip its history but tail it from here on.
                if offsets[path] == nil, let modified = values.contentModificationDate, modified < cutoff {
                    offsets[path] = size
                    continue
                }

                offsets[path] = offset + read(url, from: offset, length: size - offset, onLine: onLine)
            }
        }
    }

    /// Returns bytes consumed. A trailing partial line is left for the next scan.
    private func read(_ url: URL, from offset: UInt64, length: UInt64, onLine: (URL, Data) -> Void) -> UInt64 {
        guard let handle = try? FileHandle(forReadingFrom: url) else { return 0 }
        defer { try? handle.close() }
        guard (try? handle.seek(toOffset: offset)) != nil,
              let data = try? handle.read(upToCount: Int(length)),
              let lastNewline = data.lastIndex(of: UInt8(ascii: "\n"))
        else { return 0 }

        let complete = data[data.startIndex...lastNewline]
        for line in complete.split(separator: UInt8(ascii: "\n"), omittingEmptySubsequences: true) {
            onLine(url, Data(line))
        }
        return UInt64(complete.count)
    }
}
