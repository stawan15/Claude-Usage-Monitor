import Foundation

enum Format {
    static func tokens(_ n: Int) -> String {
        let v = Double(n)
        switch abs(v) {
        case 1_000_000_000...: return String(format: "%.2fB", v / 1_000_000_000)
        case 1_000_000...: return String(format: "%.1fM", v / 1_000_000)
        case 1_000...: return String(format: "%.1fK", v / 1_000)
        default: return "\(n)"
        }
    }

    static func cost(_ usd: Double) -> String {
        usd < 10 ? String(format: "$%.2f", usd) : String(format: "$%.0f", usd)
    }

    /// "—" when nothing in the totals could be priced.
    static func cost(_ totals: Totals) -> String {
        totals.pricedRequests == 0 ? "—" : cost(totals.cost)
    }

    static func time(_ date: Date) -> String {
        date.formatted(date: .omitted, time: .shortened)
    }

    static func duration(_ interval: TimeInterval) -> String {
        let minutes = max(0, Int(interval / 60))
        if minutes >= 24 * 60 { return "\(minutes / (24 * 60))d \(minutes % (24 * 60) / 60)h" }
        return minutes >= 60 ? "\(minutes / 60)h \(minutes % 60)m" : "\(minutes)m"
    }

    /// 300 → "5h", 10080 → "weekly", 43200 → "30d".
    static func window(minutes: Int) -> String {
        switch minutes {
        case 10080: "weekly"
        case let m where m % 1440 == 0: "\(m / 1440)d"
        case let m where m % 60 == 0: "\(m / 60)h"
        default: "\(minutes)m"
        }
    }

    /// "claude-opus-5-5" → "Opus 5.5", "claude-haiku-4-5-20251001" → "Haiku 4.5".
    /// Other vendors' ids are kept as-is, minus any "provider/" prefix.
    static func modelName(_ id: String) -> String {
        let bare = id.split(separator: "/").last.map(String.init) ?? id
        guard bare.hasPrefix("claude-") else { return bare }
        var parts = bare.split(separator: "-").map(String.init)
        parts.removeFirst()
        parts.removeAll { $0.count == 8 && $0.allSatisfy(\.isNumber) }
        guard let family = parts.first else { return id }
        let version = parts.dropFirst().joined(separator: ".")
        return family.capitalized + (version.isEmpty ? "" : " " + version)
    }
}
