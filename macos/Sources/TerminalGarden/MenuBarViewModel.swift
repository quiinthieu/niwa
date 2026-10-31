import Foundation
import Observation

/// Thin-client state coordinator for the macOS `MenuBarExtra` companion (`PLAN.md` §3, §6, §7 & §10 T14).
///
/// Owns zero simulation rules and performs zero disk writes: reads `snapshot.json`, watches for
/// atomic `rename(2)` updates via `SnapshotWatcher` (`DispatchSourceFileSystemObject`), samples
/// macOS idle time in-process on a coalesced 60-second timer (`ADR 003`), and delegates all
/// mutations and config reads/writes to the `niwa` CLI.
@MainActor
@Observable
public final class MenuBarViewModel {
    /// Power-budget justification (`PLAN.md` §0.7 & §7):
    /// - One garden tick is 60 seconds (`TICK_SECS = 60`). Waking at most once every 60 seconds
    ///   with a 6.0-second (10%) tolerance lets macOS coalesce the timer with system wakes and
    ///   keep the `LSUIElement` agent in App Nap with zero filesystem polling between ticks.
    public static let tickIntervalSeconds: TimeInterval = 60.0
    public static let tickToleranceSeconds: TimeInterval = 6.0

    public let paths: NiwaDataPaths
    public let metadata: ProductMetadata

    private let cli: any NiwaCLIRunning
    private let idleSampler: any IdleTimeSampling
    private let loginItemManager: any LoginItemManaging
    private let terminalLauncher: any TerminalLaunching
    private let nowProvider: @Sendable () -> Int64

    @ObservationIgnored
    private nonisolated(unsafe) var watcher: SnapshotWatcher?
    @ObservationIgnored
    private nonisolated(unsafe) var periodicTimer: Timer?
    @ObservationIgnored
    private var cachedSettingsViewModel: SettingsViewModel?
    @ObservationIgnored
    private var settingsWindowController: SettingsWindowController?
    private var didAttemptInitialBootstrap = false

    public private(set) var snapshot: GardenSnapshot?
    public private(set) var snapshotError: String?
    public private(set) var isNiwaMissing: Bool = false
    public private(set) var searchedBinaryPaths: [String] = []
    public private(set) var signalsPaused: Bool = false
    public private(set) var activitySignalEnabled: Bool = false
    public private(set) var anySignalEnabled: Bool = false
    public private(set) var showMenuBarTitle: Bool = true
    public private(set) var launchAtLoginStatus: LaunchAtLoginStatus = .notRegistered
    public private(set) var statusBannerMessage: String?

    public init(
        paths: NiwaDataPaths = .resolve(),
        cli: any NiwaCLIRunning = ProcessNiwaCLI(),
        idleSampler: any IdleTimeSampling = CoreGraphicsIdleTimeSampler(),
        loginItemManager: any LoginItemManaging = SMAppServiceLoginItemManager(),
        terminalLauncher: any TerminalLaunching = ProcessTerminalLauncher(),
        metadata: ProductMetadata = .current,
        nowProvider: @escaping @Sendable () -> Int64 = { Int64(Date().timeIntervalSince1970) }
    ) {
        self.paths = paths
        self.cli = cli
        self.idleSampler = idleSampler
        self.loginItemManager = loginItemManager
        self.terminalLauncher = terminalLauncher
        self.metadata = metadata
        self.nowProvider = nowProvider
        self.launchAtLoginStatus = loginItemManager.currentStatus()
    }

    deinit {
        periodicTimer?.invalidate()
        watcher?.stop()
    }

    /// Single-cell compact glyph shown in the macOS menu bar title (`PLAN.md` §4.3 & §10 T14).
    public var menuBarTitle: String {
        guard showMenuBarTitle else {
            return ""
        }
        return snapshot?.glyph ?? "◌"
    }

    /// Homebrew install command surfaced when `niwa` is absent (`PLAN.md` §1.1 G6 & §10 T14 AC 2).
    public var missingNiwaInstallCommand: String {
        "brew install quiinthieu/tap/\(metadata.homebrewFormula)"
    }

    /// Helpful guidance shown in the popover when `niwa` cannot be found (`PLAN.md` §10 T14 AC 2).
    public var missingNiwaGuidance: String {
        "The '\(metadata.cliBin)' CLI binary was not found. Install it with Homebrew (\(missingNiwaInstallCommand)) or set NIWA_BIN to a custom binary path."
    }

    /// Starts the zero-polling `DispatchSource` watcher on `snapshot.json`, synchronizes settings
    /// from `niwa config get`, loads `snapshot.json`, and schedules the 60-second coalesced timer.
    public func start() {
        syncFromSystem()

        if watcher == nil {
            let newWatcher = SnapshotWatcher(paths: paths) { [weak self] in
                Task { @MainActor [weak self] in
                    self?.reloadSnapshotFromDisk()
                }
            }
            newWatcher.start()
            watcher = newWatcher
        }

        if periodicTimer == nil {
            // Power budget (PLAN §0.7 & §7): single 60 s timer with 6.0 s coalescing tolerance.
            let timer = Timer.scheduledTimer(
                withTimeInterval: Self.tickIntervalSeconds,
                repeats: true
            ) { [weak self] _ in
                Task { @MainActor [weak self] in
                    self?.handlePeriodicTick()
                }
            }
            timer.tolerance = Self.tickToleranceSeconds
            periodicTimer = timer
        }
    }

    public func stop() {
        periodicTimer?.invalidate()
        periodicTimer = nil
        watcher?.stop()
        watcher = nil
    }

    /// Synchronizes CLI availability, `config.toml` keys (via `niwa config get`), Login Item
    /// status, and `snapshot.json`.
    public func syncFromSystem() {
        launchAtLoginStatus = loginItemManager.currentStatus()
        syncConfigFromCLI()
        reloadSnapshotFromDisk()

        if snapshot == nil, !isNiwaMissing, !didAttemptInitialBootstrap {
            didAttemptInitialBootstrap = true
            if (try? cli.refreshStatusSnapshot()) != nil {
                reloadSnapshotFromDisk()
            }
        }
    }

    public func retryLocateNiwa() {
        didAttemptInitialBootstrap = false
        statusBannerMessage = nil
        syncFromSystem()
    }

    /// Reads `snapshot.json` directly from disk without invoking `niwa`.
    public func reloadSnapshotFromDisk() {
        let fileURL = paths.snapshotFile
        guard FileManager.default.fileExists(atPath: fileURL.path) else {
            snapshot = nil
            snapshotError = nil
            return
        }

        do {
            let bytes = try Data(contentsOf: fileURL)
            snapshot = try GardenSnapshot.decode(from: bytes)
            snapshotError = nil
        } catch {
            snapshotError = error.localizedDescription
        }
    }

    /// Reads menu-bar-relevant settings strictly through `niwa config get` (`PLAN.md` §6).
    public func syncConfigFromCLI() {
        guard cli.locateBinary() != nil else {
            isNiwaMissing = true
            searchedBinaryPaths = cli.searchedCandidatePaths()
            return
        }

        isNiwaMissing = false
        searchedBinaryPaths = []

        if let pausedVal = try? cli.getConfig(key: "signals.paused") {
            signalsPaused = (pausedVal == "true")
        }
        let activityOn = (try? cli.getConfig(key: "signals.activity.enabled")) == "true"
        let gitOn = (try? cli.getConfig(key: "signals.git.enabled")) == "true"
        let shellOn = (try? cli.getConfig(key: "signals.shell.enabled")) == "true"

        activitySignalEnabled = activityOn
        anySignalEnabled = activityOn || gitOn || shellOn

        if let showTitleVal = try? cli.getConfig(key: "menubar.show_title") {
            showMenuBarTitle = (showTitleVal != "false")
        }
    }

    /// Executes one 60-second tick: samples `CGEventSource` idle time when `signals.activity` is
    /// active (`ADR 003`), or refreshes stale time-only snapshots (`>= 60 s`) without waking
    /// unnecessarily when `snapshot.json` is already fresh (`PLAN.md` §7).
    public func handlePeriodicTick() {
        syncConfigFromCLI()
        guard !isNiwaMissing else { return }

        reloadSnapshotFromDisk()
        let now = nowProvider()

        do {
            if !signalsPaused && activitySignalEnabled {
                let sampledIdle = idleSampler.sampleIdleSeconds()
                _ = try cli.runDaemonTick(idleSeconds: sampledIdle)
                reloadSnapshotFromDisk()
            } else if !signalsPaused && anySignalEnabled {
                _ = try cli.runDaemonTick(idleSeconds: nil)
                reloadSnapshotFromDisk()
            } else if snapshot == nil || !(snapshot?.isFresh(nowUnixSeconds: now) ?? false) {
                _ = try cli.refreshStatusSnapshot()
                reloadSnapshotFromDisk()
            }
        } catch {
            statusBannerMessage = error.localizedDescription
        }
    }

    /// Runs `niwa harvest --all` and refreshes the rendered snapshot (`PLAN.md` §10 T14).
    public func harvestAll() {
        guard !isNiwaMissing else { return }
        do {
            let outcome = try cli.harvestAll()
            reloadSnapshotFromDisk()
            let message = outcome.stdout.trimmingCharacters(in: .whitespacesAndNewlines)
            statusBannerMessage = message.isEmpty ? nil : message
        } catch {
            handleCLIActionError(error)
        }
    }

    /// Launches the interactive `niwa` TUI in macOS `Terminal.app` (`PLAN.md` §10 T14).
    public func openTUIInTerminal() {
        guard let binaryURL = cli.locateBinary() else {
            isNiwaMissing = true
            searchedBinaryPaths = cli.searchedCandidatePaths()
            return
        }
        do {
            try terminalLauncher.openInteractiveTUI(binaryURL: binaryURL)
            statusBannerMessage = nil
        } catch {
            statusBannerMessage = error.localizedDescription
        }
    }

    /// Toggles `signals.paused` via `niwa config set signals.paused <true|false>` (`PLAN.md` §6 & §10 T14).
    public func togglePauseSignals() {
        guard !isNiwaMissing else { return }
        let target = !signalsPaused
        do {
            _ = try cli.setConfig(key: "signals.paused", value: target ? "true" : "false")
            signalsPaused = target
            statusBannerMessage = target
                ? "Work signals paused."
                : "Work signals resumed."
        } catch {
            handleCLIActionError(error)
        }
    }

    /// Registers or unregisters Launch at Login via `SMAppService.mainApp` (`PLAN.md` §10 T14).
    public func setLaunchAtLogin(_ enabled: Bool) {
        do {
            if enabled {
                try loginItemManager.register()
            } else {
                try loginItemManager.unregister()
            }
            launchAtLoginStatus = loginItemManager.currentStatus()
            statusBannerMessage = nil
        } catch {
            launchAtLoginStatus = loginItemManager.currentStatus()
            statusBannerMessage = error.localizedDescription
        }
    }

    public func openLoginItemsSettings() {
        loginItemManager.openSystemSettingsLoginItems()
    }

    /// Shared `SettingsViewModel` wired to refresh menu bar state whenever `niwa config set` runs.
    public var settingsViewModel: SettingsViewModel {
        if let existing = cachedSettingsViewModel {
            return existing
        }
        let created = SettingsViewModel(
            cli: cli,
            consentCatalog: .current,
            metadata: metadata,
            onConfigChanged: { [weak self] in
                self?.syncFromSystem()
            }
        )
        cachedSettingsViewModel = created
        return created
    }

    /// Opens and focuses the native macOS Settings window (`PLAN.md` §10 T15).
    public func openSettingsWindow() {
        if settingsWindowController == nil {
            settingsWindowController = SettingsWindowController(viewModel: settingsViewModel)
        }
        settingsWindowController?.showWindow()
    }

    private func handleCLIActionError(_ error: any Error) {
        if case NiwaCLIError.binaryMissing(_, let searched) = error {
            isNiwaMissing = true
            searchedBinaryPaths = searched
        }
        statusBannerMessage = error.localizedDescription
    }
}
