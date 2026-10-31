#if canImport(XCTest)
import XCTest
#endif
import Foundation

final class FakeIdleSampler: IdleTimeSampling, @unchecked Sendable {
    var nextIdleSeconds: Double?

    init(nextIdleSeconds: Double?) {
        self.nextIdleSeconds = nextIdleSeconds
    }

    func sampleIdleSeconds() -> Double? {
        nextIdleSeconds
    }
}

final class FakeLoginItemManager: LoginItemManaging, @unchecked Sendable {
    var status: LaunchAtLoginStatus = .notRegistered
    var simulatedPostRegisterStatus: LaunchAtLoginStatus = .enabled
    var registerCalls = 0
    var unregisterCalls = 0
    var openSettingsCalls = 0

    func currentStatus() -> LaunchAtLoginStatus {
        status
    }

    func register() throws {
        registerCalls += 1
        status = simulatedPostRegisterStatus
    }

    func unregister() throws {
        unregisterCalls += 1
        status = .notRegistered
    }

    func openSystemSettingsLoginItems() {
        openSettingsCalls += 1
    }
}

final class SpyTerminalLauncher: TerminalLaunching, @unchecked Sendable {
    var launchedBinaries: [URL] = []

    func openInteractiveTUI(binaryURL: URL) throws {
        launchedBinaries.append(binaryURL)
    }
}

final class NiwaCLIWrapperTests: XCTestCase {
    func testProcessNiwaCLILocatesStubExecutesActionsAndPropagatesEnvironment() throws {
        let fm = FileManager.default
        let tempRoot = fm.temporaryDirectory
            .appendingPathComponent("niwa-cli-test-\(UUID().uuidString)", isDirectory: true)
        let binDir = tempRoot.appendingPathComponent("bin", isDirectory: true)
        let niwaHome = tempRoot.appendingPathComponent("home", isDirectory: true)
        try fm.createDirectory(at: binDir, withIntermediateDirectories: true)
        try fm.createDirectory(at: niwaHome, withIntermediateDirectories: true)
        defer { try? fm.removeItem(at: tempRoot) }

        let metadata = ProductMetadata.current
        let logFile = tempRoot.appendingPathComponent("invocations.log")
        let stubURL = binDir.appendingPathComponent(metadata.cliBin)
        try Self.installStubBinary(at: stubURL, logFile: logFile)

        let env: [String: String] = [
            "NIWA_HOME": niwaHome.path,
            "PATH": binDir.path,
        ]
        let cli = ProcessNiwaCLI(
            environment: env,
            homeDirectory: tempRoot,
            metadata: metadata,
            includeStandardDirectories: false
        )

        let located = cli.locateBinary()
        XCTAssertEqual(located?.standardizedFileURL, stubURL.standardizedFileURL)

        let pausedBefore = try cli.getConfig(key: "signals.paused")
        XCTAssertEqual(pausedBefore, "false")

        _ = try cli.setConfig(key: "signals.paused", value: "true")
        let pausedAfter = try cli.getConfig(key: "signals.paused")
        XCTAssertEqual(pausedAfter, "true")

        let harvestOut = try cli.harvestAll()
        XCTAssertTrue(harvestOut.stdout.contains("Harvested Moss"))

        _ = try cli.runDaemonTick(idleSeconds: 14.25)

        let logged = try String(contentsOf: logFile, encoding: .utf8)
        XCTAssertTrue(logged.contains("NIWA_HOME=\(niwaHome.path) ARGS=config get signals.paused"))
        XCTAssertTrue(logged.contains("NIWA_HOME=\(niwaHome.path) ARGS=config set signals.paused true"))
        XCTAssertTrue(logged.contains("NIWA_HOME=\(niwaHome.path) ARGS=harvest --all"))
        XCTAssertTrue(logged.contains("NIWA_HOME=\(niwaHome.path) ARGS=daemon --once --idle-secs 14.250"))
    }

    func testProcessNiwaCLIReportsBinaryMissingWhenNotFound() {
        let fm = FileManager.default
        let emptyDir = fm.temporaryDirectory
            .appendingPathComponent("niwa-missing-\(UUID().uuidString)", isDirectory: true)
        try? fm.createDirectory(at: emptyDir, withIntermediateDirectories: true)
        defer { try? fm.removeItem(at: emptyDir) }

        let metadata = ProductMetadata.current
        let cli = ProcessNiwaCLI(
            environment: ["PATH": emptyDir.path],
            homeDirectory: emptyDir,
            metadata: metadata,
            includeStandardDirectories: false
        )

        XCTAssertNil(cli.locateBinary())
        XCTAssertThrowsError(try cli.harvestAll()) { error in
            guard case NiwaCLIError.binaryMissing(let binName, let searched) = error else {
                XCTFail("Expected binaryMissing error, got \(error)")
                return
            }
            XCTAssertEqual(binName, metadata.cliBin)
            XCTAssertEqual(
                searched,
                [emptyDir.appendingPathComponent(metadata.cliBin).path]
            )
        }
    }

    @MainActor
    func testViewModelCoordinatesHarvestPauseTerminalLaunchLoginItemsAndIdleSampler() throws {
        let fm = FileManager.default
        let tempRoot = fm.temporaryDirectory
            .appendingPathComponent("niwa-vm-test-\(UUID().uuidString)", isDirectory: true)
        let binDir = tempRoot.appendingPathComponent("bin", isDirectory: true)
        let niwaHome = tempRoot.appendingPathComponent("home", isDirectory: true)
        try fm.createDirectory(at: binDir, withIntermediateDirectories: true)
        try fm.createDirectory(at: niwaHome, withIntermediateDirectories: true)
        defer { try? fm.removeItem(at: tempRoot) }

        let metadata = ProductMetadata.current
        let logFile = tempRoot.appendingPathComponent("vm-invocations.log")
        let stubURL = binDir.appendingPathComponent(metadata.cliBin)
        try Self.installStubBinary(at: stubURL, logFile: logFile)

        // Write an initial blooming snapshot in `niwaHome`.
        let paths = NiwaDataPaths(dataDirectory: niwaHome)
        try Data("""
        {
          "schema_version": 1,
          "glyph": "◈",
          "summary": "2/8 planted · 1 blooming",
          "season": "spring",
          "compact_frame": "Niwa · Spring · 2/8 planted · 1 blooming\\n  1. ◈ Moss (Blooming)",
          "updated_at": 1752000000,
          "dormant": false,
          "planted_count": 2,
          "blooming_count": 1
        }
        """.utf8).write(to: paths.snapshotFile)

        let cli = ProcessNiwaCLI(
            environment: [
                "NIWA_HOME": niwaHome.path,
                "NIWA_BIN": stubURL.path,
            ],
            homeDirectory: tempRoot,
            metadata: metadata,
            includeStandardDirectories: false
        )
        let idleSampler = FakeIdleSampler(nextIdleSeconds: 9.5)
        let loginManager = FakeLoginItemManager()
        let terminalSpy = SpyTerminalLauncher()

        let viewModel = MenuBarViewModel(
            paths: paths,
            cli: cli,
            idleSampler: idleSampler,
            loginItemManager: loginManager,
            terminalLauncher: terminalSpy,
            metadata: metadata,
            nowProvider: { 1_752_000_060 }
        )

        viewModel.syncFromSystem()
        let initialMissing = viewModel.isNiwaMissing
        let initialTitle = viewModel.menuBarTitle
        let initialActivity = viewModel.activitySignalEnabled
        XCTAssertFalse(initialMissing)
        XCTAssertEqual(initialTitle, "◈")
        XCTAssertTrue(initialActivity)

        // 1. Periodic 60 s tick samples `CGEventSource` idle time and invokes `daemon --once --idle-secs`.
        viewModel.handlePeriodicTick()

        // 2. Harvest All invokes `niwa harvest --all` and reloads the updated snapshot written by `niwa`.
        viewModel.harvestAll()
        let postHarvestGlyph = viewModel.snapshot?.glyph
        let postHarvestBlooming = viewModel.snapshot?.bloomingCount
        XCTAssertEqual(postHarvestGlyph, "·")
        XCTAssertEqual(postHarvestBlooming, 0)

        // 3. Open TUI launches `Terminal.app` with the resolved `niwa` binary URL.
        viewModel.openTUIInTerminal()
        XCTAssertEqual(terminalSpy.launchedBinaries.count, 1)
        XCTAssertEqual(
            terminalSpy.launchedBinaries.first?.standardizedFileURL,
            stubURL.standardizedFileURL
        )

        // 4. Pause / Resume signals writes strictly through `niwa config set signals.paused`.
        viewModel.togglePauseSignals()
        let pausedState = viewModel.signalsPaused
        XCTAssertTrue(pausedState)
        viewModel.togglePauseSignals()
        let resumedState = viewModel.signalsPaused
        XCTAssertFalse(resumedState)

        // 5. Launch at Login toggles `SMAppService` and handles `.requiresApproval`.
        loginManager.simulatedPostRegisterStatus = .requiresApproval
        viewModel.setLaunchAtLogin(true)
        let approvalStatus = viewModel.launchAtLoginStatus
        XCTAssertEqual(approvalStatus, .requiresApproval)
        viewModel.openLoginItemsSettings()
        XCTAssertEqual(loginManager.openSettingsCalls, 1)
        viewModel.setLaunchAtLogin(false)
        let disabledStatus = viewModel.launchAtLoginStatus
        XCTAssertEqual(disabledStatus, .notRegistered)

        let logged = try String(contentsOf: logFile, encoding: .utf8)
        XCTAssertTrue(logged.contains("ARGS=daemon --once --idle-secs 9.500"))
        XCTAssertTrue(logged.contains("ARGS=harvest --all"))
        XCTAssertTrue(logged.contains("ARGS=config set signals.paused true"))
        XCTAssertTrue(logged.contains("ARGS=config set signals.paused false"))
    }

    static func installStubBinary(at stubURL: URL, logFile: URL) throws {
        let script = """
        #!/bin/sh
        set -eu
        STATE_PAUSE="${NIWA_HOME}/paused.state"
        printf 'NIWA_HOME=%s ARGS=%s\\n' "${NIWA_HOME:-}" "$*" >> "\(logFile.path)"

        if [ "${1:-}" = "config" ] && [ "${2:-}" = "get" ]; then
          case "${3:-}" in
            signals.paused)
              if [ -f "$STATE_PAUSE" ]; then /bin/cat "$STATE_PAUSE"; else echo "false"; fi
              ;;
            signals.activity.enabled)
              echo "true"
              ;;
            signals.git.enabled|signals.shell.enabled)
              echo "false"
              ;;
            menubar.show_title)
              echo "true"
              ;;
            *)
              echo ""
              ;;
          esac
          exit 0
        fi

        if [ "${1:-}" = "config" ] && [ "${2:-}" = "set" ]; then
          if [ "${3:-}" = "signals.paused" ]; then
            printf '%s\\n' "${4:-false}" > "$STATE_PAUSE"
          fi
          printf '%s = %s\\n' "${3:-}" "${4:-}"
          exit 0
        fi

        if [ "${1:-}" = "harvest" ]; then
          /bin/cat > "${NIWA_HOME}/snapshot.json" <<'EOF'
        {
          "schema_version": 1,
          "glyph": "·",
          "summary": "1/8 planted · 0 blooming",
          "season": "spring",
          "compact_frame": "Niwa · Spring · 1/8 planted · 0 blooming\\n  2. · Fern (Seed)",
          "updated_at": 1752000060,
          "dormant": false,
          "planted_count": 1,
          "blooming_count": 0
        }
        EOF
          echo "Harvested Moss from plot 1 (seeds: Fern)."
          exit 0
        fi

        if [ "${1:-}" = "daemon" ] || [ "${1:-}" = "status" ]; then
          exit 0
        fi

        exit 0
        """
        try Data(script.utf8).write(to: stubURL)
        try FileManager.default.setAttributes(
            [.posixPermissions: NSNumber(value: Int16(0o755))],
            ofItemAtPath: stubURL.path
        )
    }
}
