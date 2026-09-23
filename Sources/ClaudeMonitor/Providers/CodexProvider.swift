import Foundation

/// OpenAI Codex CLI: `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`.
///
/// Each turn logs an `event_msg` of type `token_count` carrying that turn's usage and the
/// account's current rate limits. The model and working directory come from earlier
/// `turn_context` / `session_meta` lines in the same file.
final class CodexProvider: UsageProvider {
    let watchPaths: [URL] = {
        let home = Paths.env("CODEX_HOME") ?? Paths.home.appendingPathComponent(".codex")
        return [home.appendingPathComponent("sessions"), home.appendingPathComponent("archived_sessions")]
    }()

    private let tailer = JSONLTailer()
    private var seen = Set<String>()
    private var models: [String: String] = [:]
    private var projects: [String: String] = [:]

    private struct Line: Decodable {
        let timestamp: String?
        let type: String?
        let payload: Payload?
    }

    private struct Payload: Decodable {
        let type: String?
        let model: String?
        let cwd: String?
        let info: Info?
        let rate_limits: RateLimits?
    }

    private struct Info: Decodable {
        let total_token_usage: TokenUsage?
        let last_token_usage: TokenUsage?
    }

    private struct TokenUsage: Decodable {
        let input_tokens: Int?
        let cached_input_tokens: Int?
        let cache_write_input_tokens: Int?
        let output_tokens: Int?
        let total_tokens: Int?
    }

    private struct RateLimits: Decodable {
        let primary: Window?
        let secondary: Window?
    }

    private struct Window: Decodable {
        let used_percent: Double?
        let window_minutes: Int?
        let resets_at: Double?
    }

    private static let markers = ["\"token_count\"", "\"turn_context\"", "\"session_meta\""].map { Data($0.utf8) }

    func scan(cutoff: Date, into output: inout ScanOutput) {
        tailer.scan(roots: watchPaths, cutoff: cutoff) { url, line in
            guard Self.markers.contains(where: { line.range(of: $0) != nil }),
                  let entry = try? JSONDecoder().decode(Line.self, from: line),
                  let payload = entry.payload
            else { return }

            // Same across sessions/ and archived_sessions/, so an archived file isn't counted twice.
            let session = url.deletingPathExtension().lastPathComponent

            switch entry.type {
            case "session_meta", "turn_context":
                if let model = payload.model { models[session] = model }
                if let cwd = payload.cwd { projects[session] = URL(fileURLWithPath: cwd).lastPathComponent }

            case "event_msg" where payload.type == "token_count":
                guard let date = Timestamp.parse(entry.timestamp), date >= cutoff else { return }

                if let limits = payload.rate_limits {
                    for window in [limits.primary, limits.secondary].compactMap({ $0 }) {
                        guard let used = window.used_percent, let minutes = window.window_minutes else { continue }
                        output.limits.append(LimitStatus(
                            tool: .codex,
                            windowMinutes: minutes,
                            usedPercent: used,
                            resetsAt: window.resets_at.map { Date(timeIntervalSince1970: $0) },
                            observedAt: date
                        ))
                    }
                }

                // Codex repeats token_count with an unchanged running total; the total dedupes it.
                guard let last = payload.info?.last_token_usage,
                      let runningTotal = payload.info?.total_token_usage?.total_tokens,
                      seen.insert("\(session):\(runningTotal)").inserted
                else { return }

                // OpenAI counts cached tokens inside input_tokens, and reasoning inside output_tokens.
                let input = last.input_tokens ?? 0
                let cached = last.cached_input_tokens ?? 0
                output.events.append(UsageEvent(
                    key: "codex:\(session):\(runningTotal)",
                    date: date,
                    tool: .codex,
                    model: models[session] ?? "codex",
                    project: projects[session] ?? "codex",
                    input: max(0, input - cached),
                    output: last.output_tokens ?? 0,
                    cacheWrite: last.cache_write_input_tokens ?? 0,
                    cacheRead: cached
                ))

            default:
                break
            }
        }
    }
}
