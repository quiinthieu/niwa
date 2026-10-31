#if canImport(XCTest)
import XCTest
#endif
import AppKit
import Foundation
import SwiftUI

final class SettingsWindowTests: XCTestCase {
    func testConsentCatalogLoadsSharedTomlAndMatchesCLIDisclosureTextByteForByte() throws {
        let catalog = ConsentCatalog.current

        XCTAssertEqual(catalog.baselineTime.displayName, "Wall time")
        XCTAssertEqual(catalog.signal(.activity).displayName, "Active time")
        XCTAssertEqual(catalog.signal(.git).displayName, "Git commits")
        XCTAssertEqual(catalog.signal(.shell).displayName, "Shell commands")

        for kind in SignalKindName.allCases {
            let disclosure = catalog.signal(kind)
            let body = disclosure.renderDisclosureBody(signal: kind)
            let fullText = disclosure.renderDisclosureText(signal: kind)

            XCTAssertTrue(body.hasPrefix("Signal: \(kind.rawValue) (\(disclosure.displayName))\n"))
            XCTAssertTrue(body.contains("What Niwa will store locally:\n"))
            XCTAssertTrue(body.contains("What Niwa NEVER stores or transmits:\n"))
            XCTAssertTrue(body.hasSuffix(disclosure.prompt))
            XCTAssertEqual(fullText, "\(body) [y/N]: ")
        }

        // Cross-check byte-for-byte parity against the compiled Rust `niwa` CLI binary whenever
        // present (in `cargo test` + `xcodebuild test` CI runs and local builds).
        if let realNiwaURL = Self.locateCompiledRustBinary() {
            let fm = FileManager.default
            let tempHome = fm.temporaryDirectory
                .appendingPathComponent("niwa-consent-parity-\(UUID().uuidString)", isDirectory: true)
            try fm.createDirectory(at: tempHome, withIntermediateDirectories: true)
            defer { try? fm.removeItem(at: tempHome) }

            for kind in SignalKindName.allCases {
                let cliStdout = try Self.runDeclinedSignalsEnable(
                    binaryURL: realNiwaURL,
                    niwaHome: tempHome,
                    signal: kind.rawValue
                )
                let expectedPrefix = catalog.signal(kind).renderDisclosureText(signal: kind)
                XCTAssertTrue(
                    cliStdout.hasPrefix(expectedPrefix),
                    "Swift disclosure for '\(kind.rawValue)' must match Rust CLI byte-for-byte.\nExpected prefix:\n\(expectedPrefix)\nActual CLI stdout:\n\(cliStdout)"
                )
            }
        }
    }

    @MainActor
    func testSettingsViewModelCoversEveryConfigKeyAndEnforcesSignalConsentGate() throws {
        let fm = FileManager.default
        let tempRoot = fm.temporaryDirectory
            .appendingPathComponent("niwa-settings-vm-\(UUID().uuidString)", isDirectory: true)
        let binDir = tempRoot.appendingPathComponent("bin", isDirectory: true)
        let niwaHome = tempRoot.appendingPathComponent("home", isDirectory: true)
        try fm.createDirectory(at: binDir, withIntermediateDirectories: true)
        try fm.createDirectory(at: niwaHome, withIntermediateDirectories: true)
        defer { try? fm.removeItem(at: tempRoot) }

        let metadata = ProductMetadata.current
        let logFile = tempRoot.appendingPathComponent("settings-invocations.log")
        let stubURL = binDir.appendingPathComponent(metadata.cliBin)
        try NiwaCLIWrapperTests.installStubBinary(
            at: stubURL,
            logFile: logFile,
            defaultActivityEnabled: false
        )

        let cli = ProcessNiwaCLI(
            environment: ["NIWA_HOME": niwaHome.path, "NIWA_BIN": stubURL.path],
            homeDirectory: tempRoot,
            metadata: metadata,
            includeStandardDirectories: false
        )

        var changeNotifications = 0
        let vm = SettingsViewModel(
            cli: cli,
            consentCatalog: .current,
            metadata: metadata,
            onConfigChanged: { changeNotifications += 1 }
        )
        vm.reloadFromCLI()

        let keyCount = SettingsViewModel.allConfigKeys.count
        let initialMissing = vm.isNiwaMissing
        let initialSprites = vm.sprites
        let initialFps = vm.fps
        let initialActivity = vm.activityEnabled
        let initialGit = vm.gitEnabled
        let initialShell = vm.shellEnabled
        XCTAssertEqual(keyCount, 17)
        XCTAssertFalse(initialMissing)
        XCTAssertEqual(initialSprites, "unicode")
        XCTAssertEqual(initialFps, 4)
        XCTAssertFalse(initialActivity)
        XCTAssertFalse(initialGit)
        XCTAssertFalse(initialShell)

        // 1-8. Appearance, terminal, seasons, dormancy, prompt, and menu bar title keys.
        vm.setSprites("ascii")
        vm.setColor("256")
        vm.setFps(2)
        vm.setSeasonMode("garden")
        vm.setHemisphere("south")

        // Invalid dormant_after_days = 0 surfaces CLI error without corrupting state.
        vm.dormantAfterDaysDraft = "0"
        vm.commitDormantAfterDaysDraft()
        let zeroDormantErr = vm.errorMessage
        let preservedDormant = vm.dormantAfterDays
        XCTAssertNotNil(zeroDormantErr)
        XCTAssertEqual(preservedDormant, 7)

        vm.dormantAfterDaysDraft = "14"
        vm.commitDormantAfterDaysDraft()
        let validDormantErr = vm.errorMessage
        let updatedDormant = vm.dormantAfterDays
        XCTAssertNil(validDormantErr)
        XCTAssertEqual(updatedDormant, 14)

        vm.setPromptStyle("emoji")
        vm.setMenubarShowTitle(false)
        vm.setSignalsPaused(true)

        // 9-15. Enabling any signal MUST first show shared consent text without calling `config set`.
        for kind in SignalKindName.allCases {
            let logBeforeRequest = (try? String(contentsOf: logFile, encoding: .utf8)) ?? ""
            vm.requestSetSignalEnabled(kind, true)

            let pendingSignal = vm.pendingConsentSignal
            let pendingBody = vm.pendingConsentBodyText
            let pendingCLI = vm.pendingConsentCLIText
            XCTAssertEqual(pendingSignal, kind)
            XCTAssertEqual(
                pendingBody,
                ConsentCatalog.current.signal(kind).renderDisclosureBody(signal: kind)
            )
            XCTAssertEqual(
                pendingCLI,
                ConsentCatalog.current.signal(kind).renderDisclosureText(signal: kind)
            )
            let logDuringConsent = try String(contentsOf: logFile, encoding: .utf8)
            XCTAssertEqual(
                logBeforeRequest,
                logDuringConsent,
                "Requesting to enable '\(kind.rawValue)' must not invoke `niwa config set` before user confirms consent"
            )

            // Cancelling leaves signal disabled and writes nothing.
            vm.cancelPendingSignalConsent()
            let pendingAfterCancel = vm.pendingConsentSignal
            XCTAssertNil(pendingAfterCancel)
            let logAfterCancel = try String(contentsOf: logFile, encoding: .utf8)
            XCTAssertEqual(logBeforeRequest, logAfterCancel)

            // Re-requesting and confirming invokes `niwa config set signals.<kind>.enabled true`.
            vm.requestSetSignalEnabled(kind, true)
            vm.confirmPendingSignalConsent()
            let pendingAfterConfirm = vm.pendingConsentSignal
            XCTAssertNil(pendingAfterConfirm)
        }

        let finalActivity = vm.activityEnabled
        let finalActivityTs = vm.activityConsentedAt
        let finalGit = vm.gitEnabled
        let finalGitTs = vm.gitConsentedAt
        let finalShell = vm.shellEnabled
        let finalShellTs = vm.shellConsentedAt
        XCTAssertTrue(finalActivity)
        XCTAssertEqual(finalActivityTs, "1752000777")
        XCTAssertTrue(finalGit)
        XCTAssertEqual(finalGitTs, "1752000778")
        XCTAssertTrue(finalShell)
        XCTAssertEqual(finalShellTs, "1752000779")

        // Explicit `signals.*.consented_at` updates and clear (`none`).
        vm.activityConsentedAtDraft = "1752001111"
        vm.commitSignalConsentedAtDraft(.activity)
        vm.gitConsentedAtDraft = "1752002222"
        vm.commitSignalConsentedAtDraft(.git)
        vm.clearSignalConsentedAt(.shell)
        let clearedShellTs = vm.shellConsentedAt
        XCTAssertEqual(clearedShellTs, "none")

        // 16 & 17. `git.repos` and `git.author_email` (including invalid email validation).
        vm.gitReposDraft = "/tmp/repo-one,/tmp/repo-two"
        vm.commitGitReposDraft()
        let savedRepos = vm.gitRepos
        XCTAssertEqual(savedRepos, "/tmp/repo-one,/tmp/repo-two")

        vm.gitAuthorEmailDraft = "not-an-email"
        vm.commitGitAuthorEmailDraft()
        let invalidEmailErr = vm.errorMessage
        XCTAssertNotNil(invalidEmailErr)

        vm.gitAuthorEmailDraft = "gardener@example.com"
        vm.commitGitAuthorEmailDraft()
        let validEmailErr = vm.errorMessage
        let savedEmail = vm.gitAuthorEmail
        XCTAssertNil(validEmailErr)
        XCTAssertEqual(savedEmail, "gardener@example.com")

        XCTAssertGreaterThan(changeNotifications, 10)

        // Verify EVERY command logged by the Settings UI was strictly `config get` or `config set`,
        // and that ALL 17 canonical `config.toml` keys were written through `niwa config set`.
        let logged = try String(contentsOf: logFile, encoding: .utf8)
        for line in logged.split(whereSeparator: \.isNewline) {
            XCTAssertTrue(
                line.contains("ARGS=config get ") || line.contains("ARGS=config set "),
                "Menu bar Settings UI must only invoke `niwa config get|set` (PLAN §6), saw: \(line)"
            )
        }
        for key in SettingsViewModel.allConfigKeys {
            XCTAssertTrue(
                logged.contains("ARGS=config set \(key) "),
                "Expected `niwa config set \(key) ...` in CLI log, got:\n\(logged)"
            )
        }
    }

    @MainActor
    func testSettingsViewAndConsentSheetRenderCleanlyInNSHostingView() throws {
        let fm = FileManager.default
        let tempRoot = fm.temporaryDirectory
            .appendingPathComponent("niwa-settings-ui-\(UUID().uuidString)", isDirectory: true)
        let binDir = tempRoot.appendingPathComponent("bin", isDirectory: true)
        let niwaHome = tempRoot.appendingPathComponent("home", isDirectory: true)
        try fm.createDirectory(at: binDir, withIntermediateDirectories: true)
        try fm.createDirectory(at: niwaHome, withIntermediateDirectories: true)
        defer { try? fm.removeItem(at: tempRoot) }

        let metadata = ProductMetadata.current
        let logFile = tempRoot.appendingPathComponent("settings-ui.log")
        let stubURL = binDir.appendingPathComponent(metadata.cliBin)
        try NiwaCLIWrapperTests.installStubBinary(
            at: stubURL,
            logFile: logFile,
            defaultActivityEnabled: false
        )

        let cli = ProcessNiwaCLI(
            environment: ["NIWA_HOME": niwaHome.path, "NIWA_BIN": stubURL.path],
            homeDirectory: tempRoot,
            metadata: metadata,
            includeStandardDirectories: false
        )
        let vm = SettingsViewModel(cli: cli, consentCatalog: .current, metadata: metadata)
        vm.reloadFromCLI()

        let hostingView = NSHostingView(rootView: SettingsView(viewModel: vm))
        hostingView.frame = NSRect(x: 0, y: 0, width: 580, height: 640)
        hostingView.layoutSubtreeIfNeeded()

        let fitting = hostingView.fittingSize
        XCTAssertGreaterThan(fitting.width, 480)
        XCTAssertGreaterThan(fitting.height, 400)

        if let bitmap = hostingView.bitmapImageRepForCachingDisplay(in: hostingView.bounds) {
            hostingView.cacheDisplay(in: hostingView.bounds, to: bitmap)
            XCTAssertGreaterThan(bitmap.pixelsWide, 0)
            XCTAssertGreaterThan(bitmap.pixelsHigh, 0)
        } else {
            XCTFail("Expected non-nil bitmapImageRepForCachingDisplay for SettingsView")
        }

        // Open Signal Consent confirmation card and re-rasterize.
        vm.requestSetSignalEnabled(.shell, true)
        hostingView.layoutSubtreeIfNeeded()
        if let consentBitmap = hostingView.bitmapImageRepForCachingDisplay(in: hostingView.bounds) {
            hostingView.cacheDisplay(in: hostingView.bounds, to: consentBitmap)
            XCTAssertGreaterThan(consentBitmap.pixelsWide, 0)
            XCTAssertGreaterThan(consentBitmap.pixelsHigh, 0)
        } else {
            XCTFail("Expected non-nil bitmapImageRepForCachingDisplay for Signal Consent view")
        }
    }

    private static func locateCompiledRustBinary(sourceFilePath: String = #filePath) -> URL? {
        let repoRoot = URL(fileURLWithPath: sourceFilePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
        let binName = ProductMetadata.current.cliBin
        let candidates = [
            URL(fileURLWithPath: "/tmp/niwa-target/debug/\(binName)"),
            repoRoot.appendingPathComponent("target/debug/\(binName)"),
        ]
        .filter { FileManager.default.isExecutableFile(atPath: $0.path) }
        .sorted { lhs, rhs in
            let lDate = (try? lhs.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate) ?? .distantPast
            let rDate = (try? rhs.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate) ?? .distantPast
            return lDate > rDate
        }

        for url in candidates where supportsSignalsSubcommand(url) {
            return url
        }
        return nil
    }

    private static func supportsSignalsSubcommand(_ binaryURL: URL) -> Bool {
        let probe = Process()
        probe.executableURL = binaryURL
        probe.arguments = ["signals", "--help"]
        probe.standardInput = FileHandle.nullDevice
        probe.standardOutput = FileHandle.nullDevice
        probe.standardError = FileHandle.nullDevice
        do {
            try probe.run()
            probe.waitUntilExit()
            return probe.terminationStatus == 0
        } catch {
            return false
        }
    }

    private static func runDeclinedSignalsEnable(
        binaryURL: URL,
        niwaHome: URL,
        signal: String
    ) throws -> String {
        let process = Process()
        process.executableURL = binaryURL
        process.arguments = ["signals", "enable", signal]
        var env = ProcessInfo.processInfo.environment
        env["NIWA_HOME"] = niwaHome.path
        process.environment = env

        let stdinPipe = Pipe()
        let stdoutPipe = Pipe()
        process.standardInput = stdinPipe
        process.standardOutput = stdoutPipe
        process.standardError = FileHandle.nullDevice

        try process.run()
        stdinPipe.fileHandleForWriting.write(Data("n\n".utf8))
        try? stdinPipe.fileHandleForWriting.close()

        let data = stdoutPipe.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()
        return String(decoding: data, as: UTF8.self)
    }
}
