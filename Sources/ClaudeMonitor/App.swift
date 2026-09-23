import AppKit

/// Plain AppKit entry point: the only UI is the status item panel, so there is no SwiftUI
/// `App` scene (a placeholder `Settings` scene would open a stray empty window).
@main
enum ClaudeMonitorApp {
    static func main() {
        MainActor.assumeIsolated {
            let app = NSApplication.shared
            let delegate = AppDelegate()
            app.delegate = delegate
            app.run()
        }
    }
}

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    private var panel: StatusPanelController?

    func applicationDidFinishLaunching(_ notification: Notification) {
        // Menu bar only. The bundled .app sets LSUIElement too; this covers `swift run`.
        NSApp.setActivationPolicy(.accessory)
        panel = StatusPanelController(store: UsageStore())
    }
}
