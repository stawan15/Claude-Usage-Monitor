import Foundation
import Observation

struct Totals: Sendable {
    var input = 0
    var output = 0
    var cacheWrite = 0
    var cacheRead = 0
    var cost = 0.0
    var requests = 0
    /// Requests with a known cost. When zero, the cost is unknown rather than free.
    var pricedRequests = 0

    var total: Int { input + output + cacheWrite + cacheRead }

    mutating func add(_ e: UsageEvent, cost c: Double?) {
        input += e.input
        output += e.output
        cacheWrite += e.cacheWrite
        cacheRead += e.cacheRead
        cost += c ?? 0
        requests += 1
        if c != nil { pricedRequests += 1 }
    }
}

struct NamedTotals: Identifiable, Sendable {
    let name: String
    let totals: Totals
    var id: String { name }
}

struct RangeStats: Sendable {
    var totals = Totals()
    var byTool: [NamedTotals] = []
    var byModel: [NamedTotals] = []
    var byProject: [NamedTotals] = []
}

/// A Claude plan usage window: starts at the top of the hour of the first request
/// after the previous window expired, and lasts five hours.
struct Block: Sendable {
    let start: Date
    let end: Date
    let totals: Totals
}

struct Snapshot: Sendable {
    var today = RangeStats()
    var week = RangeStats()
    /// Claude Code only; nil when filtered to another tool or no window is active.
    var block: Block?
    var burnPerMinute = 0.0
    var lastActivity: Date?
    var unpricedModels: [String] = []
    var limits: [LimitStatus] = []
    /// Tools with any usage in the last 7 days, regardless of the filter.
    var tools: [AgentTool] = []
    /// Today across all tools, for the menu bar.
    var todayAllTools = 0
}

@MainActor
@Observable
final class UsageStore {
    static let blockLength: TimeInterval = 5 * 3600
    static let burnWindow: TimeInterval = 15 * 60

    private(set) var snapshot = Snapshot()
    private(set) var loaded = false

    /// nil shows every tool.
    var filter: AgentTool? {
        didSet { recompute() }
    }

    @ObservationIgnored private var events: [UsageEvent] = []
    @ObservationIgnored private var limits: [String: LimitStatus] = [:]
    @ObservationIgnored private let pricing = Pricing.load()
    @ObservationIgnored private var watcher: UsageWatcher?
    @ObservationIgnored private var clock: Timer?

    init() {
        let watcher = UsageWatcher { [weak self] output, initial in
            Task { @MainActor in self?.ingest(output, initial: initial) }
        }
        watcher.start()
        self.watcher = watcher

        // Time-based figures (5h window, burn rate, "today") change even without new logs.
        clock = Timer.scheduledTimer(withTimeInterval: 30, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.recompute() }
        }
    }

    private func ingest(_ output: ScanOutput, initial: Bool) {
        if initial { loaded = true }
        events.append(contentsOf: output.events)
        events.sort { $0.date < $1.date }
        for limit in output.limits where limit.observedAt >= (limits[limit.id]?.observedAt ?? .distantPast) {
            limits[limit.id] = limit
        }
        recompute()
    }

    private func recompute(now: Date = .now) {
        events.removeAll { $0.date < now.addingTimeInterval(-UsageWatcher.retention) }

        let calendar = Calendar.current
        let startOfToday = calendar.startOfDay(for: now)
        let startOfWeek = calendar.date(byAdding: .day, value: -6, to: startOfToday)!
        let showBlock = filter == nil || filter == .claudeCode

        var today = Accumulator()
        var week = Accumulator()
        var unpriced = Set<String>()
        var tools = Set<AgentTool>()
        var todayAllTools = 0
        var lastActivity: Date?
        var blockStart: Date?
        var blockTotals = Totals()
        var burnTokens = 0

        for event in events {
            if event.date >= startOfWeek { tools.insert(event.tool) }
            if event.date >= startOfToday { todayAllTools += event.total }

            let cost = event.cost ?? pricing.cost(of: event)

            // Plan windows are a Claude concept; other tools report their own limits.
            if showBlock, event.tool == .claudeCode {
                if let start = blockStart, event.date < start.addingTimeInterval(Self.blockLength) {
                    blockTotals.add(event, cost: cost)
                } else {
                    blockStart = calendar.dateInterval(of: .hour, for: event.date)?.start ?? event.date
                    blockTotals = Totals()
                    blockTotals.add(event, cost: cost)
                }
            }

            guard filter == nil || event.tool == filter else { continue }
            if cost == nil { unpriced.insert(Format.modelName(event.model)) }
            lastActivity = event.date
            if event.date >= startOfWeek { week.add(event, cost: cost) }
            if event.date >= startOfToday { today.add(event, cost: cost) }
            if event.date >= now.addingTimeInterval(-Self.burnWindow) { burnTokens += event.total }
        }

        var block: Block?
        if let start = blockStart, now < start.addingTimeInterval(Self.blockLength) {
            block = Block(start: start, end: start.addingTimeInterval(Self.blockLength), totals: blockTotals)
        }

        snapshot = Snapshot(
            today: today.stats,
            week: week.stats,
            block: block,
            burnPerMinute: Double(burnTokens) / (Self.burnWindow / 60),
            lastActivity: lastActivity,
            unpricedModels: unpriced.sorted(),
            limits: limits.values
                .filter { ($0.resetsAt ?? .distantFuture) > now && (filter == nil || $0.tool == filter) }
                .sorted { ($0.tool.rawValue, $0.windowMinutes) < ($1.tool.rawValue, $1.windowMinutes) },
            tools: AgentTool.allCases.filter(tools.contains),
            todayAllTools: todayAllTools
        )
    }
}

private struct Accumulator {
    var totals = Totals()
    var tools: [String: Totals] = [:]
    var models: [String: Totals] = [:]
    var projects: [String: Totals] = [:]

    mutating func add(_ e: UsageEvent, cost: Double?) {
        totals.add(e, cost: cost)
        tools[e.tool.name, default: Totals()].add(e, cost: cost)
        models[Format.modelName(e.model), default: Totals()].add(e, cost: cost)
        projects[e.project, default: Totals()].add(e, cost: cost)
    }

    var stats: RangeStats {
        func ranked(_ dict: [String: Totals]) -> [NamedTotals] {
            dict.map { NamedTotals(name: $0.key, totals: $0.value) }
                .sorted { $0.totals.total > $1.totals.total }
        }
        return RangeStats(totals: totals, byTool: ranked(tools), byModel: ranked(models), byProject: ranked(projects))
    }
}
