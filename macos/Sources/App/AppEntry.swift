import SwiftUI

/// Application entry point hosting Niwa's macOS `MenuBarExtra` companion (`PLAN.md` §3.1 & §10 T14).
@main
struct NiwaMenuBarApp: App {
    private let viewModel: MenuBarViewModel

    init() {
        let model = MenuBarViewModel()
        model.start()
        self.viewModel = model
    }

    var body: some Scene {
        MenuBarExtra {
            MenuBarPopoverView(viewModel: viewModel)
        } label: {
            if viewModel.showMenuBarTitle, !viewModel.menuBarTitle.isEmpty {
                Text(viewModel.menuBarTitle)
                    .font(.system(.body, design: .monospaced))
            } else {
                Image(systemName: "leaf")
                    .accessibilityLabel(viewModel.metadata.displayName)
            }
        }
        .menuBarExtraStyle(.window)

        Settings {
            SettingsView(viewModel: viewModel.settingsViewModel)
        }
    }
}
