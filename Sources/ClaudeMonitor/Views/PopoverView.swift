import ServiceManagement
import SwiftUI

struct PopoverView: View {
    let store: UsageStore

    enum Range: String, CaseIterable, Identifiable {
        case today = "Today"
        case week = "7 days"
        var id: Self { self }
    }

    @AppStorage("range") private var range: Range = .today

    private var snapshot: Snapshot { store.snapshot }
    private var stats: RangeStats { range == .today ? snapshot.today : snapshot.week }

    var body: some View {
        // Refresh every 30 s so countdowns and "live" state stay current while open.
        TimelineView(.periodic(from: .now, by: 30)) { context in
            VStack(alignment: .leading, spacing: 14) {
                header(now: context.date)
                if snapshot.tools.count > 1 { toolPicker }
                if store.filter == nil || store.filter == .claudeCode, snapshot.tools.contains(.claudeCode) || snapshot.block != nil {
                    BlockSection(block: snapshot.block, burnPerMinute: snapshot.burnPerMinute, now: context.date)
                }
                if !snapshot.limits.isEmpty {
                    LimitsSection(limits: snapshot.limits, now: context.date)
                }
                rangePicker
                usage
                if store.filter == nil, stats.byTool.count > 1 {
                    breakdown("Tools", stats.byTool, limit: 4)
                }
                breakdown("Models", stats.byModel, limit: 5)
                breakdown("Projects", stats.byProject, limit: 6)

                if !snapshot.unpricedModels.isEmpty {
                    Text("— no price for \(snapshot.unpricedModels.joined(separator: ", "))")
                        .font(Theme.mono(10))
                        .foregroundStyle(Theme.subtle)
                        .lineLimit(1)
                        .truncationMode(.tail)
                        .help("Add prices to ~/.config/claude-monitor/pricing.json")
                }

                Footer()
            }
        }
        .font(Theme.mono())
        .padding(14)
        .frame(width: 380)
        .background(Theme.background)
    }

    private func header(now: Date) -> some View {
        let live = snapshot.lastActivity.map { now.timeIntervalSince($0) < 60 } ?? false
        return TermBox {
            VStack(alignment: .leading, spacing: 4) {
                HStack(spacing: 6) {
                    Spinner(active: !store.loaded || live)
                    Text("AI Usage Monitor").fontWeight(.semibold).foregroundStyle(Theme.text)
                }
                Group {
                    if !store.loaded {
                        Text("Reading logs…")
                    } else if let last = snapshot.lastActivity {
                        HStack(spacing: 0) {
                            Text(live ? "● live" : "○ idle")
                                .foregroundStyle(live ? Theme.success : Theme.dim)
                            Text(" · last activity \(Format.time(last))")
                        }
                    } else {
                        Text("No activity in the last 7 days")
                    }
                }
                .foregroundStyle(Theme.dim)
                .padding(.leading, 20)
            }
        }
    }

    private var toolPicker: some View {
        TabRow(
            options: [nil] + snapshot.tools.map(Optional.some),
            selection: store.filter,
            title: { $0?.shortName ?? "All" },
            select: { store.filter = $0 }
        )
    }

    private var rangePicker: some View {
        TabRow(options: Range.allCases, selection: range, title: \.rawValue, select: { range = $0 }) {
            Text("tab ⇥").foregroundStyle(Theme.subtle)
        }
        .background {
            // Tab switches range, like cycling options in Claude Code.
            Button("") { range = range == .today ? .week : .today }
                .keyboardShortcut(.tab, modifiers: [])
                .hidden()
        }
    }

    private var usage: some View {
        let t = stats.totals
        return TermSection(title: "Usage", trailing: "\(t.requests) req") {
            StatRow(label: "total", value: Format.tokens(t.total), detail: Format.cost(t), emphasized: true)
            StatRow(label: "input", value: Format.tokens(t.input))
            StatRow(label: "output", value: Format.tokens(t.output))
            StatRow(label: "cache write", value: Format.tokens(t.cacheWrite))
            StatRow(label: "cache read", value: Format.tokens(t.cacheRead))
        }
    }

    @ViewBuilder
    private func breakdown(_ title: String, _ rows: [NamedTotals], limit: Int) -> some View {
        if !rows.isEmpty {
            TermSection(title: title, bullet: Theme.dim) {
                ForEach(rows.prefix(limit)) { row in
                    StatRow(label: row.name, value: Format.tokens(row.totals.total), detail: Format.cost(row.totals))
                }
            }
        }
    }
}

private struct BlockSection: View {
    let block: Block?
    let burnPerMinute: Double
    let now: Date

    var body: some View {
        TermSection(title: "5-hour window", bullet: block == nil ? Theme.dim : Theme.success) {
            if let block {
                let elapsed = now.timeIntervalSince(block.start) / UsageStore.blockLength
                StatRow(label: "used", value: Format.tokens(block.totals.total), detail: Format.cost(block.totals), emphasized: true)
                HStack(spacing: 8) {
                    TextBar(fraction: elapsed)
                    Text("\(Int(min(elapsed, 1) * 100))%").foregroundStyle(Theme.dim)
                }
                .padding(.vertical, 2)
                StatRow(label: "window", value: "\(Format.time(block.start)) → \(Format.time(block.end))")
                StatRow(label: "resets in", value: Format.duration(block.end.timeIntervalSince(now)))
                HStack {
                    Text("burn rate").foregroundStyle(Theme.dim)
                    Spacer()
                    Text("\(Format.tokens(Int(burnPerMinute))) tok/min")
                        .foregroundStyle(burnPerMinute > 0 ? Theme.claude : Theme.dim)
                }
            } else {
                Text("No active window. It starts with your next request.")
                    .foregroundStyle(Theme.dim)
            }
        }
    }
}

/// Plan limits the tools report themselves, e.g. Codex's `rate_limits`.
private struct LimitsSection: View {
    let limits: [LimitStatus]
    let now: Date

    var body: some View {
        TermSection(title: "Plan limits", bullet: Theme.warning) {
            ForEach(limits) { limit in
                VStack(alignment: .leading, spacing: 1) {
                    HStack {
                        Text("\(limit.tool.shortName) · \(Format.window(minutes: limit.windowMinutes))")
                            .foregroundStyle(Theme.dim)
                        Spacer()
                        Text("\(Int(limit.usedPercent.rounded()))% used")
                            .foregroundStyle(limit.usedPercent >= 90 ? Theme.warning : Theme.text)
                    }
                    HStack(spacing: 8) {
                        TextBar(fraction: limit.usedPercent / 100, width: 22)
                        if let resets = limit.resetsAt {
                            Text("resets \(Format.duration(resets.timeIntervalSince(now)))")
                                .foregroundStyle(Theme.dim)
                        }
                    }
                }
            }
        }
    }
}

/// `❯ Selected   Other   Other` option row.
private struct TabRow<Option: Hashable, Trailing: View>: View {
    let options: [Option]
    let selection: Option
    let title: (Option) -> String
    let select: (Option) -> Void
    @ViewBuilder var trailing: Trailing

    var body: some View {
        HStack(spacing: 14) {
            ForEach(options, id: \.self) { option in
                let selected = option == selection
                Button {
                    select(option)
                } label: {
                    Text((selected ? "❯ " : "  ") + title(option))
                        .fontWeight(selected ? .semibold : .regular)
                        .foregroundStyle(selected ? Theme.claude : Theme.dim)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
            Spacer()
            trailing
        }
    }
}

extension TabRow where Trailing == EmptyView {
    init(options: [Option], selection: Option, title: @escaping (Option) -> String, select: @escaping (Option) -> Void) {
        self.init(options: options, selection: selection, title: title, select: select) { EmptyView() }
    }
}

private struct Footer: View {
    @State private var launchAtLogin = SMAppService.mainApp.status == .enabled
    @State private var loginError: String?

    /// SMAppService only works from a bundled .app, not from `swift run`.
    private let isBundled = Bundle.main.bundleURL.pathExtension == "app"

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Rectangle().fill(Theme.subtle).frame(height: 1)
            HStack {
                Button {
                    setLaunchAtLogin(!launchAtLogin)
                } label: {
                    HStack(spacing: 0) {
                        Text(launchAtLogin ? "[✓]" : "[ ]")
                            .foregroundStyle(launchAtLogin ? Theme.success : Theme.dim)
                        Text(" launch at login").foregroundStyle(Theme.dim)
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .disabled(!isBundled)
                .opacity(isBundled ? 1 : 0.5)
                .help(isBundled ? "" : "Build the .app with scripts/bundle.sh to enable")

                Spacer()

                Button {
                    NSApp.terminate(nil)
                } label: {
                    HStack(spacing: 0) {
                        Text("⌘Q").foregroundStyle(Theme.text)
                        Text(" quit").foregroundStyle(Theme.dim)
                    }
                }
                .buttonStyle(.plain)
                .keyboardShortcut("q")
            }
            if let loginError {
                Text("✗ \(loginError)").foregroundStyle(.red)
            }
        }
    }

    private func setLaunchAtLogin(_ enabled: Bool) {
        do {
            if enabled {
                try SMAppService.mainApp.register()
            } else {
                try SMAppService.mainApp.unregister()
            }
            loginError = nil
        } catch {
            loginError = error.localizedDescription
        }
        launchAtLogin = SMAppService.mainApp.status == .enabled
    }
}
