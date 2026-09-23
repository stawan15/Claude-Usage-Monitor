import Foundation
import SQLite3

/// OpenCode: `~/.local/share/opencode/opencode.db` (SQLite). Each assistant row in `message`
/// has provider, model, tokens and OpenCode's own cost estimate in its JSON `data` column.
/// Covers every provider OpenCode talks to: OpenAI, Google, Anthropic, OpenRouter, etc.
final class OpenCodeProvider: UsageProvider {
    private let dataDir = (Paths.env("XDG_DATA_HOME") ?? Paths.home.appendingPathComponent(".local/share"))
        .appendingPathComponent("opencode")

    var watchPaths: [URL] { [dataDir] }

    private var db: OpaquePointer?
    /// Highest `time_updated` (ms) already read.
    private var cursor: Int64?
    private var seen = Set<String>()

    deinit { sqlite3_close(db) }

    private struct Row: Decodable {
        let modelID: String?
        let providerID: String?
        let cost: Double?
        let tokens: Tokens?
        let path: PathInfo?
        let time: Time?
    }

    private struct Tokens: Decodable {
        let input: Int?
        let output: Int?
        let reasoning: Int?
        let cache: Cache?
    }

    private struct Cache: Decodable {
        let read: Int?
        let write: Int?
    }

    private struct PathInfo: Decodable {
        let cwd: String?
    }

    private struct Time: Decodable {
        let created: Double?
        let completed: Double?
    }

    func scan(cutoff: Date, into output: inout ScanOutput) {
        guard let db = open() else { return }
        let since = cursor ?? Int64(cutoff.timeIntervalSince1970 * 1000)

        // Rows only get tokens once the response completes; incomplete ones are picked up
        // on a later scan because completing them bumps time_updated past the cursor.
        let sql = """
            SELECT id, time_updated, data FROM message
            WHERE time_updated > ?
              AND json_extract(data, '$.role') = 'assistant'
              AND json_extract(data, '$.time.completed') IS NOT NULL
            ORDER BY time_updated
            """
        var statement: OpaquePointer?
        guard sqlite3_prepare_v2(db, sql, -1, &statement, nil) == SQLITE_OK else { return }
        defer { sqlite3_finalize(statement) }
        sqlite3_bind_int64(statement, 1, since)

        var newest = since
        while sqlite3_step(statement) == SQLITE_ROW {
            newest = max(newest, sqlite3_column_int64(statement, 1))
            guard let idText = sqlite3_column_text(statement, 0),
                  let blob = sqlite3_column_blob(statement, 2)
            else { continue }

            let id = String(cString: idText)
            let json = Data(bytes: blob, count: Int(sqlite3_column_bytes(statement, 2)))
            guard !seen.contains(id),
                  let row = try? JSONDecoder().decode(Row.self, from: json),
                  let tokens = row.tokens,
                  let millis = row.time?.completed ?? row.time?.created
            else { continue }

            let event = UsageEvent(
                key: "opencode:" + id,
                date: Date(timeIntervalSince1970: millis / 1000),
                tool: .openCode,
                model: row.modelID ?? row.providerID ?? "unknown",
                project: row.path?.cwd.map { URL(fileURLWithPath: $0).lastPathComponent } ?? "opencode",
                input: tokens.input ?? 0,
                output: (tokens.output ?? 0) + (tokens.reasoning ?? 0),
                cacheWrite: tokens.cache?.write ?? 0,
                cacheRead: tokens.cache?.read ?? 0,
                cost: row.cost
            )
            // Failed requests are stored with all-zero tokens.
            guard event.total > 0, event.date >= cutoff else { continue }
            seen.insert(id)
            output.events.append(event)
        }
        cursor = newest
    }

    private func open() -> OpaquePointer? {
        if let db { return db }
        let path = dataDir.appendingPathComponent("opencode.db").path
        guard FileManager.default.fileExists(atPath: path) else { return nil }
        var handle: OpaquePointer?
        guard sqlite3_open_v2(path, &handle, SQLITE_OPEN_READONLY, nil) == SQLITE_OK else {
            sqlite3_close(handle)
            return nil
        }
        sqlite3_busy_timeout(handle, 200)
        db = handle
        return handle
    }
}
