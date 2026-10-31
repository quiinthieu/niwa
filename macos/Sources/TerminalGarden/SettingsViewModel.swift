import Foundation
import Observation

/// Thin-client Settings coordinator covering every `PLAN.md` §6 / `AppConfig::ALL_KEYS` key
/// strictly through `niwa config get` and `niwa config set`, and gating opt-in signal activation
/// behind the shared `data/consent.toml` disclosures (`PLAN.md` §5, §6 & §10 T15).
@MainActor
@Observable
public final class SettingsViewModel {
    /// Canonical 17 dot-keys matching Rust `tg_store::config::AppConfig::ALL_KEYS`.
    public static let allConfigKeys: [String] = [
        "sprites",
        "color",
        "fps",
        "season_mode",
        "hemisphere",
        "dormant_after_days",
        "signals.paused",
        "signals.activity.enabled",
        "signals.activity.consented_at",
        "signals.git.enabled",
        "signals.git.consented_at",
        "signals.shell.enabled",
        "signals.shell.consented_at",
        "git.repos",
        "git.author_email",
        "prompt.style",
        "menubar.show_title",
    ]

    private let cli: any NiwaCLIRunning
    public let consentCatalog: ConsentCatalog
    public let metadata: ProductMetadata
    private let onConfigChanged: (@MainActor () -> Void)?

    public private(set) var isNiwaMissing: Bool = false
    public private(set) var sprites: String = "unicode"
    public private(set) var color: String = "auto"
    public private(set) var fps: Int = 4
    public private(set) var seasonMode: String = "calendar"
    public private(set) var hemisphere: String = "north"
    public private(set) var dormantAfterDays: Int = 7
    public var dormantAfterDaysDraft: String = "7"

    public private(set) var promptStyle: String = "glyph"
    public private(set) var menubarShowTitle: Bool = true
    public private(set) var signalsPaused: Bool = false

    public private(set) var activityEnabled: Bool = false
    public private(set) var activityConsentedAt: String = "none"
    public var activityConsentedAtDraft: String = "none"

    public private(set) var gitEnabled: Bool = false
    public private(set) var gitConsentedAt: String = "none"
    public var gitConsentedAtDraft: String = "none"

    public private(set) var shellEnabled: Bool = false
    public private(set) var shellConsentedAt: String = "none"
    public var shellConsentedAtDraft: String = "none"

    public private(set) var gitRepos: String = ""
    public var gitReposDraft: String = ""

    public private(set) var gitAuthorEmail: String = ""
    public var gitAuthorEmailDraft: String = ""

    public private(set) var pendingConsentSignal: SignalKindName?
    public private(set) var errorMessage: String?
    public private(set) var statusMessage: String?

    public init(
        cli: any NiwaCLIRunning = ProcessNiwaCLI(),
        consentCatalog: ConsentCatalog = .current,
        metadata: ProductMetadata = .current,
        onConfigChanged: (@MainActor () -> Void)? = nil
    ) {
        self.cli = cli
        self.consentCatalog = consentCatalog
        self.metadata = metadata
        self.onConfigChanged = onConfigChanged
    }

    /// Shared disclosure body (without stdin `[y/N]: `) for the pending signal consent sheet.
    public var pendingConsentBodyText: String? {
        guard let signal = pendingConsentSignal else { return nil }
        return consentCatalog.signal(signal).renderDisclosureBody(signal: signal)
    }

    /// Full CLI consent disclosure text (`... [y/N]: `) matching `niwa signals enable <name>`.
    public var pendingConsentCLIText: String? {
        guard let signal = pendingConsentSignal else { return nil }
        return consentCatalog.signal(signal).renderDisclosureText(signal: signal)
    }

    /// Loads all 17 `config.toml` keys strictly via `niwa config get <key>` (`PLAN.md` §6).
    public func reloadFromCLI() {
        guard cli.locateBinary() != nil else {
            isNiwaMissing = true
            return
        }
        isNiwaMissing = false
        errorMessage = nil

        if let val = try? cli.getConfig(key: "sprites"), !val.isEmpty {
            sprites = val
        }
        if let val = try? cli.getConfig(key: "color"), !val.isEmpty {
            color = val
        }
        if let val = try? cli.getConfig(key: "fps"), let parsed = Int(val) {
            fps = parsed
        }
        if let val = try? cli.getConfig(key: "season_mode"), !val.isEmpty {
            seasonMode = val
        }
        if let val = try? cli.getConfig(key: "hemisphere"), !val.isEmpty {
            hemisphere = val
        }
        if let val = try? cli.getConfig(key: "dormant_after_days"), let parsed = Int(val) {
            dormantAfterDays = parsed
            dormantAfterDaysDraft = String(parsed)
        }
        if let val = try? cli.getConfig(key: "prompt.style"), !val.isEmpty {
            promptStyle = val
        }
        if let val = try? cli.getConfig(key: "menubar.show_title") {
            menubarShowTitle = (val != "false")
        }
        if let val = try? cli.getConfig(key: "signals.paused") {
            signalsPaused = (val == "true")
        }

        activityEnabled = ((try? cli.getConfig(key: "signals.activity.enabled")) == "true")
        if let val = try? cli.getConfig(key: "signals.activity.consented_at"), !val.isEmpty {
            activityConsentedAt = val
            activityConsentedAtDraft = val
        }

        gitEnabled = ((try? cli.getConfig(key: "signals.git.enabled")) == "true")
        if let val = try? cli.getConfig(key: "signals.git.consented_at"), !val.isEmpty {
            gitConsentedAt = val
            gitConsentedAtDraft = val
        }

        shellEnabled = ((try? cli.getConfig(key: "signals.shell.enabled")) == "true")
        if let val = try? cli.getConfig(key: "signals.shell.consented_at"), !val.isEmpty {
            shellConsentedAt = val
            shellConsentedAtDraft = val
        }

        if let val = try? cli.getConfig(key: "git.repos") {
            gitRepos = val
            gitReposDraft = val
        }
        if let val = try? cli.getConfig(key: "git.author_email") {
            gitAuthorEmail = val
            gitAuthorEmailDraft = val
        }
    }

    public func setSprites(_ value: String) {
        if writeConfig(key: "sprites", value: value) {
            sprites = value
        }
    }

    public func setColor(_ value: String) {
        if writeConfig(key: "color", value: value) {
            color = value
        }
    }

    public func setFps(_ value: Int) {
        let clamped = max(1, min(4, value))
        if writeConfig(key: "fps", value: String(clamped)) {
            fps = clamped
        }
    }

    public func setSeasonMode(_ value: String) {
        if writeConfig(key: "season_mode", value: value) {
            seasonMode = value
        }
    }

    public func setHemisphere(_ value: String) {
        if writeConfig(key: "hemisphere", value: value) {
            hemisphere = value
        }
    }

    public func setDormantAfterDays(_ days: Int) {
        let text = String(days)
        if writeConfig(key: "dormant_after_days", value: text) {
            dormantAfterDays = days
            dormantAfterDaysDraft = text
        }
    }

    public func commitDormantAfterDaysDraft() {
        let trimmed = dormantAfterDaysDraft.trimmingCharacters(in: .whitespacesAndNewlines)
        if writeConfig(key: "dormant_after_days", value: trimmed),
           let parsed = Int(trimmed)
        {
            dormantAfterDays = parsed
            dormantAfterDaysDraft = String(parsed)
        }
    }

    public func setPromptStyle(_ value: String) {
        if writeConfig(key: "prompt.style", value: value) {
            promptStyle = value
        }
    }

    public func setMenubarShowTitle(_ show: Bool) {
        if writeConfig(key: "menubar.show_title", value: show ? "true" : "false") {
            menubarShowTitle = show
        }
    }

    public func setSignalsPaused(_ paused: Bool) {
        if writeConfig(key: "signals.paused", value: paused ? "true" : "false") {
            signalsPaused = paused
        }
    }

    public func isSignalEnabled(_ signal: SignalKindName) -> Bool {
        switch signal {
        case .activity: return activityEnabled
        case .git: return gitEnabled
        case .shell: return shellEnabled
        }
    }

    public func consentedAt(for signal: SignalKindName) -> String {
        switch signal {
        case .activity: return activityConsentedAt
        case .git: return gitConsentedAt
        case .shell: return shellConsentedAt
        }
    }

    /// Requests enabling or disabling an opt-in work signal.
    ///
    /// Enabling a disabled signal opens the shared `data/consent.toml` disclosure confirmation
    /// without invoking `niwa config set` until the user explicitly confirms (`PLAN.md` §10 T15 AC 2).
    public func requestSetSignalEnabled(_ signal: SignalKindName, _ enabled: Bool) {
        if enabled {
            if isSignalEnabled(signal) {
                return
            }
            errorMessage = nil
            pendingConsentSignal = signal
        } else {
            if pendingConsentSignal == signal {
                pendingConsentSignal = nil
            }
            if writeConfig(key: "signals.\(signal.rawValue).enabled", value: "false") {
                setLocalSignalEnabled(signal, false)
            }
        }
    }

    /// Confirms the pending signal consent disclosure and writes `signals.<name>.enabled = true`
    /// through `niwa config set` (which stamps `consented_at`).
    public func confirmPendingSignalConsent() {
        guard let signal = pendingConsentSignal else { return }
        pendingConsentSignal = nil

        let enabledKey = "signals.\(signal.rawValue).enabled"
        let consentKey = "signals.\(signal.rawValue).consented_at"
        if writeConfig(key: enabledKey, value: "true") {
            setLocalSignalEnabled(signal, true)
            if let stamped = try? cli.getConfig(key: consentKey), !stamped.isEmpty {
                setLocalSignalConsentedAt(signal, stamped)
            }
        }
    }

    /// Dismisses the pending signal consent disclosure without modifying `config.toml`.
    public func cancelPendingSignalConsent() {
        pendingConsentSignal = nil
    }

    public func commitSignalConsentedAtDraft(_ signal: SignalKindName) {
        let draft: String
        switch signal {
        case .activity: draft = activityConsentedAtDraft
        case .git: draft = gitConsentedAtDraft
        case .shell: draft = shellConsentedAtDraft
        }
        let trimmed = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        let normalized = trimmed.isEmpty ? "none" : trimmed
        if writeConfig(key: "signals.\(signal.rawValue).consented_at", value: normalized) {
            setLocalSignalConsentedAt(signal, normalized)
        }
    }

    public func clearSignalConsentedAt(_ signal: SignalKindName) {
        if writeConfig(key: "signals.\(signal.rawValue).consented_at", value: "none") {
            setLocalSignalConsentedAt(signal, "none")
        }
    }

    public func commitGitReposDraft() {
        let trimmed = gitReposDraft.trimmingCharacters(in: .whitespacesAndNewlines)
        let valueToSend = trimmed.isEmpty ? "none" : trimmed
        if writeConfig(key: "git.repos", value: valueToSend) {
            let refreshed = (try? cli.getConfig(key: "git.repos")) ?? (valueToSend == "none" ? "" : valueToSend)
            gitRepos = refreshed
            gitReposDraft = refreshed
        }
    }

    public func clearGitRepos() {
        gitReposDraft = ""
        commitGitReposDraft()
    }

    public func appendGitRepoPath(_ folderPath: String) {
        let trimmedPath = folderPath.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedPath.isEmpty else { return }
        var parts = gitReposDraft
            .split(separator: ",")
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty && $0 != "none" }
        if !parts.contains(trimmedPath) {
            parts.append(trimmedPath)
        }
        gitReposDraft = parts.joined(separator: ",")
        commitGitReposDraft()
    }

    public func commitGitAuthorEmailDraft() {
        let trimmed = gitAuthorEmailDraft.trimmingCharacters(in: .whitespacesAndNewlines)
        let valueToSend = trimmed.isEmpty ? "none" : trimmed
        if writeConfig(key: "git.author_email", value: valueToSend) {
            let refreshed = (try? cli.getConfig(key: "git.author_email")) ?? (valueToSend == "none" ? "" : valueToSend)
            gitAuthorEmail = refreshed
            gitAuthorEmailDraft = refreshed
        }
    }

    public func clearGitAuthorEmail() {
        gitAuthorEmailDraft = ""
        commitGitAuthorEmailDraft()
    }

    private func setLocalSignalEnabled(_ signal: SignalKindName, _ enabled: Bool) {
        switch signal {
        case .activity: activityEnabled = enabled
        case .git: gitEnabled = enabled
        case .shell: shellEnabled = enabled
        }
    }

    private func setLocalSignalConsentedAt(_ signal: SignalKindName, _ value: String) {
        switch signal {
        case .activity:
            activityConsentedAt = value
            activityConsentedAtDraft = value
        case .git:
            gitConsentedAt = value
            gitConsentedAtDraft = value
        case .shell:
            shellConsentedAt = value
            shellConsentedAtDraft = value
        }
    }

    @discardableResult
    private func writeConfig(key: String, value: String) -> Bool {
        do {
            _ = try cli.setConfig(key: key, value: value)
            isNiwaMissing = false
            errorMessage = nil
            statusMessage = "Saved \(key) = \(value)"
            onConfigChanged?()
            return true
        } catch {
            if case NiwaCLIError.binaryMissing = error {
                isNiwaMissing = true
            }
            errorMessage = error.localizedDescription
            return false
        }
    }
}
