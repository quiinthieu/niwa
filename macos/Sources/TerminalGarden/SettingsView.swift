import AppKit
import SwiftUI

/// Native SwiftUI Settings window covering every `PLAN.md` §6 `config.toml` key and presenting
/// the shared `data/consent.toml` disclosure before enabling any work signal (`PLAN.md` §10 T15).
public struct SettingsView: View {
    public let viewModel: SettingsViewModel

    public init(viewModel: SettingsViewModel) {
        self.viewModel = viewModel
    }

    public var body: some View {
        ScrollView(.vertical, showsIndicators: true) {
            VStack(alignment: .leading, spacing: 16) {
                headerRow

                if viewModel.isNiwaMissing {
                    missingBinaryBanner
                }

                if let errorText = viewModel.formattedErrorMessage {
                    Label(errorText, systemImage: "exclamationmark.triangle")
                        .font(.caption)
                        .foregroundStyle(.red)
                        .padding(10)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(.red.opacity(0.08), in: RoundedRectangle(cornerRadius: 8))
                        .accessibilityIdentifier("settings-error-banner")
                }

                if let pendingSignal = viewModel.pendingConsentSignal,
                   let consentBody = viewModel.pendingConsentBodyText
                {
                    consentConfirmationCard(signal: pendingSignal, disclosureBody: consentBody)
                }

                appearanceSection
                seasonsAndRestSection
                signalsAndPrivacySection
                gitConfigurationSection

                if let status = viewModel.statusMessage, !status.isEmpty {
                    Text(status)
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                        .accessibilityIdentifier("settings-status-text")
                }
            }
            .padding(18)
        }
        .frame(minWidth: 540, idealWidth: 580, minHeight: 560)
    }

    private var headerRow: some View {
        HStack {
            VStack(alignment: .leading, spacing: 2) {
                Text("\(viewModel.metadata.displayName) Settings")
                    .font(.title3.weight(.semibold))
                Text("All settings write locally to config.toml through `\(viewModel.metadata.cliBin) config set`.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer()
            Button("Reload") {
                viewModel.reloadFromCLI()
            }
            .controlSize(.small)
            .accessibilityIdentifier("reload-settings-button")
        }
    }

    private var missingBinaryBanner: some View {
        Label(
            "'\(viewModel.metadata.cliBin)' CLI binary not found. Install it with `brew install quiinthieu/tap/\(viewModel.metadata.homebrewFormula)`.",
            systemImage: "wrench.and.screwdriver"
        )
        .font(.caption)
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.quaternary.opacity(0.45), in: RoundedRectangle(cornerRadius: 8))
    }

    private func consentConfirmationCard(
        signal: SignalKindName,
        disclosureBody: String
    ) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            Label(
                "Local Storage Consent Required (\(signal.rawValue))",
                systemImage: "hand.raised"
            )
            .font(.subheadline.weight(.semibold))

            Text(disclosureBody)
                .font(.system(.caption, design: .monospaced))
                .textSelection(.enabled)
                .padding(10)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(.quaternary.opacity(0.4), in: RoundedRectangle(cornerRadius: 6))
                .accessibilityIdentifier("signal-consent-disclosure-text")

            HStack {
                Spacer()
                Button("Cancel") {
                    viewModel.cancelPendingSignalConsent()
                }
                .accessibilityIdentifier("cancel-signal-consent-button")

                Button("Enable Signal") {
                    viewModel.confirmPendingSignalConsent()
                }
                .buttonStyle(.borderedProminent)
                .accessibilityIdentifier("confirm-signal-consent-button")
            }
        }
        .padding(12)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 10))
        .overlay(
            RoundedRectangle(cornerRadius: 10)
                .strokeBorder(.secondary.opacity(0.35), lineWidth: 1)
        )
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("signal-consent-card")
    }

    private var appearanceSection: some View {
        GroupBox("Appearance & Terminal") {
            VStack(alignment: .leading, spacing: 10) {
                Picker(
                    "Sprite set (sprites)",
                    selection: Binding(
                        get: { viewModel.sprites },
                        set: { viewModel.setSprites($0) }
                    )
                ) {
                    Text("Unicode").tag("unicode")
                    Text("7-bit ASCII").tag("ascii")
                }
                .accessibilityIdentifier("sprites-picker")

                Picker(
                    "Terminal color tier (color)",
                    selection: Binding(
                        get: { viewModel.color },
                        set: { viewModel.setColor($0) }
                    )
                ) {
                    Text("Auto").tag("auto")
                    Text("Truecolor (24-bit)").tag("truecolor")
                    Text("256-color").tag("256")
                    Text("16-color").tag("16")
                    Text("None (monochrome)").tag("none")
                }
                .accessibilityIdentifier("color-picker")

                Picker(
                    "TUI active frame rate cap (fps)",
                    selection: Binding(
                        get: { viewModel.fps },
                        set: { viewModel.setFps($0) }
                    )
                ) {
                    Text("1 fps").tag(1)
                    Text("2 fps").tag(2)
                    Text("3 fps").tag(3)
                    Text("4 fps").tag(4)
                }
                .accessibilityIdentifier("fps-picker")

                Picker(
                    "Shell prompt glyph style (prompt.style)",
                    selection: Binding(
                        get: { viewModel.promptStyle },
                        set: { viewModel.setPromptStyle($0) }
                    )
                ) {
                    Text("Stage glyph").tag("glyph")
                    Text("Emoji").tag("emoji")
                    Text("None").tag("none")
                }
                .accessibilityIdentifier("prompt-style-picker")

                Toggle(
                    "Show stage glyph in macOS menu bar (menubar.show_title)",
                    isOn: Binding(
                        get: { viewModel.menubarShowTitle },
                        set: { viewModel.setMenubarShowTitle($0) }
                    )
                )
                .accessibilityIdentifier("menubar-show-title-toggle")
            }
            .padding(8)
        }
    }

    private var seasonsAndRestSection: some View {
        GroupBox("Seasons & Rest") {
            VStack(alignment: .leading, spacing: 10) {
                Picker(
                    "Season mode (season_mode)",
                    selection: Binding(
                        get: { viewModel.seasonMode },
                        set: { viewModel.setSeasonMode($0) }
                    )
                ) {
                    Text("Real calendar").tag("calendar")
                    Text("Garden cycle").tag("garden")
                }
                .accessibilityIdentifier("season-mode-picker")

                Picker(
                    "Hemisphere (hemisphere)",
                    selection: Binding(
                        get: { viewModel.hemisphere },
                        set: { viewModel.setHemisphere($0) }
                    )
                ) {
                    Text("Northern").tag("north")
                    Text("Southern").tag("south")
                }
                .accessibilityIdentifier("hemisphere-picker")

                HStack(spacing: 8) {
                    Text("Dormant after inactive days (dormant_after_days)")
                    Spacer()
                    TextField(
                        "Days",
                        text: Binding(
                            get: { viewModel.dormantAfterDaysDraft },
                            set: { viewModel.dormantAfterDaysDraft = $0 }
                        )
                    )
                    .frame(width: 72)
                    .onSubmit {
                        viewModel.commitDormantAfterDaysDraft()
                    }
                    .accessibilityIdentifier("dormant-after-days-field")

                    Stepper(
                        "",
                        value: Binding(
                            get: { viewModel.dormantAfterDays },
                            set: { viewModel.setDormantAfterDays(max(1, $0)) }
                        ),
                        in: 1...365
                    )
                    .labelsHidden()

                    Button("Save") {
                        viewModel.commitDormantAfterDaysDraft()
                    }
                    .controlSize(.small)
                }
            }
            .padding(8)
        }
    }

    private var signalsAndPrivacySection: some View {
        GroupBox("Work Signals & Local Privacy") {
            VStack(alignment: .leading, spacing: 12) {
                Text("Wall time is always on. Work signals are opt-in, local-only, and never store command text, paths, or commit messages.")
                    .font(.caption)
                    .foregroundStyle(.secondary)

                Toggle(
                    "Temporarily pause all opt-in work signals (signals.paused)",
                    isOn: Binding(
                        get: { viewModel.signalsPaused },
                        set: { viewModel.setSignalsPaused($0) }
                    )
                )
                .accessibilityIdentifier("signals-paused-toggle")

                Divider()

                signalRow(
                    signal: .activity,
                    draftBinding: Binding(
                        get: { viewModel.activityConsentedAtDraft },
                        set: { viewModel.activityConsentedAtDraft = $0 }
                    )
                )
                Divider()
                signalRow(
                    signal: .git,
                    draftBinding: Binding(
                        get: { viewModel.gitConsentedAtDraft },
                        set: { viewModel.gitConsentedAtDraft = $0 }
                    )
                )
                Divider()
                signalRow(
                    signal: .shell,
                    draftBinding: Binding(
                        get: { viewModel.shellConsentedAtDraft },
                        set: { viewModel.shellConsentedAtDraft = $0 }
                    )
                )
            }
            .padding(8)
        }
    }

    private func signalRow(
        signal: SignalKindName,
        draftBinding: Binding<String>
    ) -> some View {
        let disclosure = viewModel.consentCatalog.signal(signal)
        return VStack(alignment: .leading, spacing: 6) {
            Toggle(
                "\(disclosure.displayName) (signals.\(signal.rawValue).enabled)",
                isOn: Binding(
                    get: { viewModel.isSignalEnabled(signal) },
                    set: { viewModel.requestSetSignalEnabled(signal, $0) }
                )
            )
            .accessibilityIdentifier("signal-\(signal.rawValue)-toggle")

            Text(disclosure.summary)
                .font(.caption2)
                .foregroundStyle(.secondary)

            HStack(spacing: 6) {
                Text("signals.\(signal.rawValue).consented_at")
                    .font(.system(.caption2, design: .monospaced))
                    .foregroundStyle(.secondary)
                Spacer()
                TextField("none or Unix secs", text: draftBinding)
                    .font(.system(.caption, design: .monospaced))
                    .frame(width: 132)
                    .onSubmit {
                        viewModel.commitSignalConsentedAtDraft(signal)
                    }
                    .accessibilityIdentifier("signal-\(signal.rawValue)-consented-at-field")

                Button("Save") {
                    viewModel.commitSignalConsentedAtDraft(signal)
                }
                .controlSize(.mini)

                Button("Clear") {
                    viewModel.clearSignalConsentedAt(signal)
                }
                .controlSize(.mini)
            }
        }
    }

    private var gitConfigurationSection: some View {
        GroupBox("Git Signal Repositories") {
            VStack(alignment: .leading, spacing: 10) {
                VStack(alignment: .leading, spacing: 4) {
                    Text("Configured local repositories (git.repos, comma-separated)")
                        .font(.caption)
                    HStack(spacing: 6) {
                        TextField(
                            "/path/to/repo-a, /path/to/repo-b",
                            text: Binding(
                                get: { viewModel.gitReposDraft },
                                set: { viewModel.gitReposDraft = $0 }
                            )
                        )
                        .font(.system(.caption, design: .monospaced))
                        .onSubmit {
                            viewModel.commitGitReposDraft()
                        }
                        .accessibilityIdentifier("git-repos-field")

                        Button("Add Folder…") {
                            chooseRepositoryFolder()
                        }
                        .controlSize(.small)

                        Button("Save") {
                            viewModel.commitGitReposDraft()
                        }
                        .controlSize(.small)

                        Button("Clear") {
                            viewModel.clearGitRepos()
                        }
                        .controlSize(.small)
                    }
                }

                VStack(alignment: .leading, spacing: 4) {
                    Text("Commit author email filter (git.author_email)")
                        .font(.caption)
                    HStack(spacing: 6) {
                        TextField(
                            "gardener@example.com",
                            text: Binding(
                                get: { viewModel.gitAuthorEmailDraft },
                                set: { viewModel.gitAuthorEmailDraft = $0 }
                            )
                        )
                        .font(.system(.caption, design: .monospaced))
                        .onSubmit {
                            viewModel.commitGitAuthorEmailDraft()
                        }
                        .accessibilityIdentifier("git-author-email-field")

                        Button("Save") {
                            viewModel.commitGitAuthorEmailDraft()
                        }
                        .controlSize(.small)

                        Button("Clear") {
                            viewModel.clearGitAuthorEmail()
                        }
                        .controlSize(.small)
                    }
                }
            }
            .padding(8)
        }
    }

    private func chooseRepositoryFolder() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = true
        panel.prompt = "Add Repository"
        if panel.runModal() == .OK {
            for url in panel.urls {
                viewModel.appendGitRepoPath(url.path)
            }
        }
    }
}
