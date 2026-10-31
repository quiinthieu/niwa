import AppKit
import SwiftUI

/// Native SwiftUI popover content for the macOS `MenuBarExtra` companion (`PLAN.md` §3.1, §4.3 & §10 T14).
public struct MenuBarPopoverView: View {
    public let viewModel: MenuBarViewModel
    private let onQuit: @MainActor () -> Void

    public init(
        viewModel: MenuBarViewModel,
        onQuit: @escaping @MainActor () -> Void = { NSApplication.shared.terminate(nil) }
    ) {
        self.viewModel = viewModel
        self.onQuit = onQuit
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            headerSection

            if viewModel.isNiwaMissing {
                missingNiwaCard
            }

            if let snapshot = viewModel.snapshot {
                compactFrameSection(snapshot: snapshot)
            } else if !viewModel.isNiwaMissing {
                emptySnapshotSection
            }

            if let errorText = viewModel.snapshotError {
                Text(errorText)
                    .font(.caption)
                    .foregroundStyle(.red)
                    .accessibilityIdentifier("snapshot-error-text")
            }

            Divider()

            actionsSection

            Divider()

            footerSection
        }
        .padding(14)
        .frame(width: 344)
    }

    private var headerSection: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Text(viewModel.snapshot?.glyph ?? "◌")
                .font(.system(.title3, design: .monospaced))
                .accessibilityHidden(true)

            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 6) {
                    Text(viewModel.metadata.displayName)
                        .font(.headline)

                    if let season = viewModel.snapshot?.season {
                        Text("· \(season.displayName)")
                            .font(.subheadline)
                            .foregroundStyle(.secondary)
                    }

                    if viewModel.snapshot?.dormant == true {
                        Text("Resting")
                            .font(.caption2.weight(.medium))
                            .padding(.horizontal, 6)
                            .padding(.vertical, 2)
                            .background(.quaternary, in: Capsule())
                            .accessibilityIdentifier("dormant-badge")
                    }
                }

                if let summary = viewModel.snapshot?.summary {
                    Text(summary)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .accessibilityIdentifier("snapshot-summary-text")
                }
            }

            Spacer(minLength: 0)
        }
    }

    private var missingNiwaCard: some View {
        VStack(alignment: .leading, spacing: 8) {
            Label("CLI Binary Not Found", systemImage: "wrench.and.screwdriver")
                .font(.subheadline.weight(.semibold))

            Text(viewModel.missingNiwaGuidance)
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)

            Text(viewModel.missingNiwaInstallCommand)
                .font(.system(.caption, design: .monospaced))
                .textSelection(.enabled)
                .padding(8)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(.quaternary.opacity(0.6), in: RoundedRectangle(cornerRadius: 6))

            if !viewModel.searchedBinaryPaths.isEmpty {
                Text("Checked: \(viewModel.searchedBinaryPaths.joined(separator: ", "))")
                    .font(.caption2)
                    .foregroundStyle(.tertiary)
                    .lineLimit(2)
            }

            HStack {
                Button("Retry") {
                    viewModel.retryLocateNiwa()
                }
                .controlSize(.small)
                .accessibilityIdentifier("retry-locate-niwa-button")

                Spacer()
            }
        }
        .padding(10)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 8))
        .overlay(
            RoundedRectangle(cornerRadius: 8)
                .strokeBorder(.quaternary, lineWidth: 1)
        )
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("missing-niwa-card")
    }

    private func compactFrameSection(snapshot: GardenSnapshot) -> some View {
        ScrollView(.vertical, showsIndicators: true) {
            Text(snapshot.compactFrame)
                .font(.system(.caption, design: .monospaced))
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(10)
                .accessibilityIdentifier("compact-frame-text")
        }
        .frame(minHeight: 84, maxHeight: 196)
        .background(.quaternary.opacity(0.35), in: RoundedRectangle(cornerRadius: 8))
    }

    private var emptySnapshotSection: some View {
        Text("No garden snapshot yet. Open the TUI to plant your first seed.")
            .font(.system(.caption, design: .monospaced))
            .foregroundStyle(.secondary)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(10)
            .background(.quaternary.opacity(0.35), in: RoundedRectangle(cornerRadius: 8))
            .accessibilityIdentifier("empty-snapshot-placeholder")
    }

    private var actionsSection: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 8) {
                Button {
                    viewModel.harvestAll()
                } label: {
                    let blooming = viewModel.snapshot?.bloomingCount ?? 0
                    let title = blooming > 0 ? "Harvest All (\(blooming))" : "Harvest All"
                    Label(title, systemImage: "sparkles")
                        .frame(maxWidth: .infinity)
                }
                .disabled(viewModel.isNiwaMissing || (viewModel.snapshot?.bloomingCount ?? 0) == 0)
                .accessibilityIdentifier("harvest-all-button")

                Button {
                    viewModel.openTUIInTerminal()
                } label: {
                    Label("Open TUI", systemImage: "terminal")
                        .frame(maxWidth: .infinity)
                }
                .disabled(viewModel.isNiwaMissing)
                .accessibilityIdentifier("open-tui-button")
            }

            Button {
                viewModel.togglePauseSignals()
            } label: {
                let label = viewModel.signalsPaused ? "Resume Work Signals" : "Pause Work Signals"
                let icon = viewModel.signalsPaused ? "play.circle" : "pause.circle"
                Label(label, systemImage: icon)
                    .frame(maxWidth: .infinity)
            }
            .disabled(viewModel.isNiwaMissing)
            .accessibilityIdentifier("pause-signals-button")

            if let banner = viewModel.statusBannerMessage, !banner.isEmpty {
                Text(banner)
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityIdentifier("status-banner-text")
            }
        }
    }

    private var footerSection: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Toggle(
                    "Launch at Login",
                    isOn: Binding(
                        get: { viewModel.launchAtLoginStatus.isToggleOn },
                        set: { viewModel.setLaunchAtLogin($0) }
                    )
                )
                .toggleStyle(.switch)
                .controlSize(.small)
                .accessibilityIdentifier("launch-at-login-toggle")

                Spacer(minLength: 8)

                Button("Settings…") {
                    viewModel.openSettingsWindow()
                }
                .buttonStyle(.plain)
                .font(.caption)
                .foregroundStyle(.secondary)
                .keyboardShortcut(",", modifiers: .command)
                .accessibilityIdentifier("open-settings-button")

                Button("Quit") {
                    onQuit()
                }
                .buttonStyle(.plain)
                .font(.caption)
                .foregroundStyle(.secondary)
                .accessibilityIdentifier("quit-button")
            }

            if viewModel.launchAtLoginStatus == .requiresApproval {
                HStack(spacing: 6) {
                    Text("Requires approval in Login Items.")
                        .font(.caption2)
                        .foregroundStyle(.secondary)

                    Button("Open Login Items…") {
                        viewModel.openLoginItemsSettings()
                    }
                    .controlSize(.mini)
                    .accessibilityIdentifier("approve-login-item-button")
                }
            }
        }
    }
}
