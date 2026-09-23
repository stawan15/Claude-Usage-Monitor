import Foundation

/// USD per million tokens.
struct ModelPrice: Codable, Sendable {
    let input: Double
    let output: Double
    let cacheWrite: Double
    let cacheRead: Double
}

/// API-equivalent cost estimates. On a Pro/Max subscription you are not billed per token;
/// this shows what the same traffic would cost on the API.
struct Pricing: Sendable {
    /// Input/output rates are Anthropic's published first-party API prices. Cache reads for
    /// Fable 5.1 and Opus 5.5 are published; the other cache rates use the standard
    /// multipliers (write = 1.25x input for the 5-minute cache, read = 0.1x input).
    static let defaults: [String: ModelPrice] = [
        "claude-fable-5-1": .init(input: 10, output: 50, cacheWrite: 12.5, cacheRead: 0.25),
        "claude-fable-5": .init(input: 10, output: 50, cacheWrite: 12.5, cacheRead: 1.0),
        "claude-mythos-5-1": .init(input: 10, output: 50, cacheWrite: 12.5, cacheRead: 1.0),
        "claude-opus-5-5": .init(input: 4, output: 20, cacheWrite: 5, cacheRead: 0.20),
        "claude-opus-5": .init(input: 5, output: 25, cacheWrite: 6.25, cacheRead: 0.5),
        "claude-opus-4-8": .init(input: 5, output: 25, cacheWrite: 6.25, cacheRead: 0.5),
        "claude-opus-4-7": .init(input: 5, output: 25, cacheWrite: 6.25, cacheRead: 0.5),
        "claude-opus-4-6": .init(input: 5, output: 25, cacheWrite: 6.25, cacheRead: 0.5),
        "claude-sonnet-5": .init(input: 2, output: 10, cacheWrite: 2.5, cacheRead: 0.2),
        "claude-sonnet-4-6": .init(input: 3, output: 15, cacheWrite: 3.75, cacheRead: 0.3),
        "claude-haiku-4-5": .init(input: 1, output: 5, cacheWrite: 1.25, cacheRead: 0.1),
    ]

    static let overrideURL = FileManager.default.homeDirectoryForCurrentUser
        .appendingPathComponent(".config/claude-monitor/pricing.json")

    private let table: [String: ModelPrice]
    /// Keys sorted longest first so "claude-opus-5-5" wins over "claude-opus-5".
    private let keys: [String]

    init(table: [String: ModelPrice]) {
        self.table = table
        self.keys = table.keys.sorted { $0.count > $1.count }
    }

    /// Built-in prices, with any entries from ~/.config/claude-monitor/pricing.json layered on top.
    static func load() -> Pricing {
        var table = defaults
        if let data = try? Data(contentsOf: overrideURL),
           let overrides = try? JSONDecoder().decode([String: ModelPrice].self, from: data) {
            table.merge(overrides) { _, new in new }
        }
        return Pricing(table: table)
    }

    func price(for model: String) -> ModelPrice? {
        keys.first { model.hasPrefix($0) }.flatMap { table[$0] }
    }

    func cost(of event: UsageEvent) -> Double? {
        guard let p = price(for: event.model) else { return nil }
        let micro = Double(event.input) * p.input
            + Double(event.output) * p.output
            + Double(event.cacheWrite) * p.cacheWrite
            + Double(event.cacheRead) * p.cacheRead
        return micro / 1_000_000
    }
}
