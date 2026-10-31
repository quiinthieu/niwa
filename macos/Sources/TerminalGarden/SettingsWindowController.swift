import AppKit
import SwiftUI

/// Presents and focuses the native Settings `NSWindow` for the `LSUIElement` menu bar companion
/// (`PLAN.md` §10 T15).
@MainActor
public final class SettingsWindowController {
    private let viewModel: SettingsViewModel
    private var window: NSWindow?

    public init(viewModel: SettingsViewModel) {
        self.viewModel = viewModel
    }

    public func showWindow() {
        viewModel.reloadFromCLI()

        let settingsWindow: NSWindow
        if let existing = window {
            settingsWindow = existing
        } else {
            let hostingController = NSHostingController(rootView: SettingsView(viewModel: viewModel))
            let created = NSWindow(contentViewController: hostingController)
            created.title = "\(viewModel.metadata.displayName) Settings"
            created.styleMask = [.titled, .closable, .miniaturizable, .resizable]
            created.setContentSize(NSSize(width: 580, height: 640))
            created.isReleasedWhenClosed = false
            created.center()
            self.window = created
            settingsWindow = created
        }

        NSApplication.shared.activate()
        settingsWindow.makeKeyAndOrderFront(nil)
        settingsWindow.orderFrontRegardless()
    }
}
