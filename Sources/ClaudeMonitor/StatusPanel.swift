import AppKit
import SwiftUI

/// Menu bar item plus a borderless panel that hugs its SwiftUI content exactly.
///
/// Used instead of `MenuBarExtra(.window)`, whose window keeps a stale height and
/// shows empty system-coloured bands above and below shorter content.
@MainActor
final class StatusPanelController {
    private let store: UsageStore
    private let statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
    private let panel: KeyablePanel
    private var clickMonitor: Any?
    private var keyMonitor: Any?

    init(store: UsageStore) {
        self.store = store

        panel = KeyablePanel(
            contentRect: .zero,
            styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered,
            defer: true
        )
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = true
        panel.level = .popUpMenu
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .transient]
        panel.isReleasedWhenClosed = false

        let hosting = FittingHostingView(rootView: AnyView(
            PopoverView(store: store)
                .clipShape(RoundedRectangle(cornerRadius: 10))
                .overlay(RoundedRectangle(cornerRadius: 10).stroke(Theme.subtle, lineWidth: 1))
        ))
        hosting.sizingOptions = [.intrinsicContentSize]
        hosting.onSizeChange = { [weak self] in self?.fitToContent() }
        panel.contentView = hosting

        if let button = statusItem.button {
            button.font = .monospacedSystemFont(ofSize: 13, weight: .regular)
            button.target = self
            button.action = #selector(toggle)
        }
        updateTitle()
    }

    /// Re-renders the menu bar title whenever today's total changes.
    private func updateTitle() {
        withObservationTracking {
            statusItem.button?.title = "✻ " + Format.tokens(store.snapshot.todayAllTools)
        } onChange: { [weak self] in
            Task { @MainActor in self?.updateTitle() }
        }
    }

    @objc private func toggle() {
        panel.isVisible ? close() : open()
    }

    private func open() {
        fitToContent()
        NSApp.activate()
        panel.makeKeyAndOrderFront(nil)

        // Close on any click outside the app, or on Escape.
        clickMonitor = NSEvent.addGlobalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown]) { [weak self] _ in
            Task { @MainActor in self?.close() }
        }
        keyMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
            guard event.keyCode == 53 else { return event } // Escape
            self?.close()
            return nil
        }
    }

    private func close() {
        panel.orderOut(nil)
        for monitor in [clickMonitor, keyMonitor].compactMap({ $0 }) {
            NSEvent.removeMonitor(monitor)
        }
        clickMonitor = nil
        keyMonitor = nil
    }

    /// Sizes the panel to its content, pinned below the status item.
    private func fitToContent() {
        guard let hosting = panel.contentView,
              let button = statusItem.button,
              let buttonWindow = button.window,
              let screen = buttonWindow.screen ?? NSScreen.main
        else { return }

        let size = hosting.fittingSize
        let anchor = buttonWindow.convertToScreen(button.convert(button.bounds, to: nil))
        let visible = screen.visibleFrame
        let x = min(max(anchor.midX - size.width / 2, visible.minX + 8), visible.maxX - size.width - 8)
        let top = anchor.minY - 6
        panel.setFrame(NSRect(x: x, y: top - size.height, width: size.width, height: size.height), display: true)
    }
}

/// Borderless panels can't become key by default, which would disable Tab and ⌘Q.
private final class KeyablePanel: NSPanel {
    override var canBecomeKey: Bool { true }
}

/// Reports SwiftUI ideal-size changes (e.g. switching tabs) so the panel can follow.
private final class FittingHostingView: NSHostingView<AnyView> {
    var onSizeChange: (() -> Void)?

    override func invalidateIntrinsicContentSize() {
        super.invalidateIntrinsicContentSize()
        DispatchQueue.main.async { [weak self] in self?.onSizeChange?() }
    }
}
