import CoreServices
import Foundation

/// Watches every provider's log location and emits new usage as it is written.
///
/// All state lives on `queue`; FSEvents callbacks and the fallback timer both run there.
final class UsageWatcher: @unchecked Sendable {
    typealias Handler = @Sendable (_ output: ScanOutput, _ initial: Bool) -> Void

    /// Ignore anything older than this; the UI shows at most 7 days.
    static let retention: TimeInterval = 8 * 24 * 3600

    private let providers: [UsageProvider]
    private let handler: Handler
    private let queue = DispatchQueue(label: "claude-monitor.watcher", qos: .utility)
    private var stream: FSEventStreamRef?
    private var watchedPaths: [String] = []
    private var timer: DispatchSourceTimer?

    init(
        providers: [UsageProvider] = [ClaudeCodeProvider(), CodexProvider(), OpenCodeProvider(), GeminiProvider()],
        handler: @escaping Handler
    ) {
        self.providers = providers
        self.handler = handler
    }

    func start() {
        queue.async {
            self.scan(initial: true)
            self.updateStream()
            self.startFallbackTimer()
        }
    }

    private func scan(initial: Bool) {
        let cutoff = Date().addingTimeInterval(-Self.retention)
        var output = ScanOutput()
        for provider in providers {
            provider.scan(cutoff: cutoff, into: &output)
        }
        if initial || !output.events.isEmpty || !output.limits.isEmpty {
            handler(output, initial)
        }
    }

    /// (Re)creates the FSEvents stream when the set of existing log directories changes,
    /// e.g. after a tool is used for the first time.
    private func updateStream() {
        let paths = providers.flatMap(\.watchPaths).map(\.path)
            .filter { FileManager.default.fileExists(atPath: $0) }
        guard paths != watchedPaths else { return }

        if let stream {
            FSEventStreamStop(stream)
            FSEventStreamInvalidate(stream)
            FSEventStreamRelease(stream)
            self.stream = nil
        }
        watchedPaths = paths
        guard !paths.isEmpty else { return }

        var context = FSEventStreamContext(
            version: 0,
            info: Unmanaged.passUnretained(self).toOpaque(),
            retain: nil, release: nil, copyDescription: nil
        )
        let callback: FSEventStreamCallback = { _, info, _, _, _, _ in
            guard let info else { return }
            Unmanaged<UsageWatcher>.fromOpaque(info).takeUnretainedValue().scan(initial: false)
        }
        // 0.25 s latency coalesces the burst of writes while a response is saved.
        guard let stream = FSEventStreamCreate(
            nil, callback, &context, paths as CFArray,
            FSEventStreamEventId(kFSEventStreamEventIdSinceNow), 0.25,
            FSEventStreamCreateFlags(kFSEventStreamCreateFlagFileEvents | kFSEventStreamCreateFlagNoDefer)
        ) else { return }

        FSEventStreamSetDispatchQueue(stream, queue)
        FSEventStreamStart(stream)
        self.stream = stream
    }

    /// Safety net for missed FSEvents, and picks up log folders created later.
    private func startFallbackTimer() {
        let timer = DispatchSource.makeTimerSource(queue: queue)
        timer.schedule(deadline: .now() + 30, repeating: 30)
        timer.setEventHandler { [weak self] in
            guard let self else { return }
            self.updateStream()
            self.scan(initial: false)
        }
        timer.resume()
        self.timer = timer
    }
}
