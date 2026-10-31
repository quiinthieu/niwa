#if canImport(XCTest)
import XCTest
#endif
import AppKit
import Foundation
import SwiftUI

final class PopoverUISmokeTests: XCTestCase {
    @MainActor
    func testPopoverRendersActiveGardenCompactFrameAndExecutesActions() throws {
        let fm = FileManager.default
        let tempRoot = fm.temporaryDirectory
            .appendingPathComponent("niwa-ui-smoke-\(UUID().uuidString)", isDirectory: true)
        let binDir = tempRoot.appendingPathComponent("bin", isDirectory: true)
        let niwaHome = tempRoot.appendingPathComponent("home", isDirectory: true)
        try fm.createDirectory(at: binDir, withIntermediateDirectories: true)
        try fm.createDirectory(at: niwaHome, withIntermediateDirectories: true)
        defer { try? fm.removeItem(at: tempRoot) }

        let metadata = ProductMetadata.current
        let logFile = tempRoot.appendingPathComponent("ui-smoke.log")
        let stubURL = binDir.appendingPathComponent(metadata.cliBin)
        try NiwaCLIWrapperTests.installStubBinary(at: stubURL, logFile: logFile)

        let paths = NiwaDataPaths(dataDirectory: niwaHome)
        try Data("""
        {
          "schema_version": 1,
          "glyph": "◈",
          "summary": "3/8 planted · 1 blooming",
          "season": "autumn",
          "compact_frame": "Niwa · Autumn · 3/8 planted · 1 blooming\\n  1. ◈ Moss (Blooming)\\n  2. ◎ Fern (Mature)",
          "updated_at": 1752000100,
          "dormant": false,
          "planted_count": 3,
          "blooming_count": 1
        }
        """.utf8).write(to: paths.snapshotFile)

        let cli = ProcessNiwaCLI(
            environment: ["NIWA_HOME": niwaHome.path, "NIWA_BIN": stubURL.path],
            homeDirectory: tempRoot,
            metadata: metadata,
            includeStandardDirectories: false
        )
        let loginManager = FakeLoginItemManager()
        let terminalSpy = SpyTerminalLauncher()

        let viewModel = MenuBarViewModel(
            paths: paths,
            cli: cli,
            idleSampler: FakeIdleSampler(nextIdleSeconds: 4.0),
            loginItemManager: loginManager,
            terminalLauncher: terminalSpy,
            metadata: metadata,
            nowProvider: { 1_752_000_110 }
        )
        viewModel.syncFromSystem()

        var quitInvoked = false
        let popover = MenuBarPopoverView(viewModel: viewModel) {
            quitInvoked = true
        }

        let hostingView = NSHostingView(rootView: popover)
        hostingView.frame = NSRect(x: 0, y: 0, width: 344, height: 420)
        hostingView.layoutSubtreeIfNeeded()

        let fittingSize = hostingView.fittingSize
        XCTAssertGreaterThan(fittingSize.width, 300)
        XCTAssertGreaterThan(fittingSize.height, 200)

        let bitmap = hostingView.bitmapImageRepForCachingDisplay(in: hostingView.bounds)
        XCTAssertNotNil(bitmap)
        if let bitmap {
            hostingView.cacheDisplay(in: hostingView.bounds, to: bitmap)
            XCTAssertGreaterThan(bitmap.pixelsWide, 0)
            XCTAssertGreaterThan(bitmap.pixelsHigh, 0)
        }

        // Trigger harvest action while hosted and confirm view re-layouts cleanly.
        viewModel.harvestAll()
        hostingView.layoutSubtreeIfNeeded()
        let updatedGlyph = viewModel.menuBarTitle
        XCTAssertEqual(updatedGlyph, "·")
        XCTAssertFalse(quitInvoked)
    }

    @MainActor
    func testPopoverRendersHelpfulMissingNiwaGuidanceCard() throws {
        let fm = FileManager.default
        let emptyRoot = fm.temporaryDirectory
            .appendingPathComponent("niwa-ui-missing-\(UUID().uuidString)", isDirectory: true)
        try fm.createDirectory(at: emptyRoot, withIntermediateDirectories: true)
        defer { try? fm.removeItem(at: emptyRoot) }

        let metadata = ProductMetadata.current
        let cli = ProcessNiwaCLI(
            environment: ["PATH": emptyRoot.path],
            homeDirectory: emptyRoot,
            metadata: metadata,
            includeStandardDirectories: false
        )
        let paths = NiwaDataPaths(dataDirectory: emptyRoot.appendingPathComponent("home"))

        let viewModel = MenuBarViewModel(
            paths: paths,
            cli: cli,
            idleSampler: FakeIdleSampler(nextIdleSeconds: nil),
            loginItemManager: FakeLoginItemManager(),
            terminalLauncher: SpyTerminalLauncher(),
            metadata: metadata
        )
        viewModel.syncFromSystem()

        let isMissing = viewModel.isNiwaMissing
        let guidance = viewModel.missingNiwaGuidance
        let installCmd = viewModel.missingNiwaInstallCommand
        let fallbackTitle = viewModel.menuBarTitle

        XCTAssertTrue(isMissing)
        XCTAssertEqual(fallbackTitle, "◌")
        XCTAssertTrue(installCmd.contains("brew install quiinthieu/tap/\(metadata.homebrewFormula)"))
        XCTAssertTrue(guidance.contains(installCmd))
        XCTAssertTrue(guidance.contains("NIWA_BIN"))

        let hostingView = NSHostingView(rootView: MenuBarPopoverView(viewModel: viewModel) {})
        hostingView.frame = NSRect(x: 0, y: 0, width: 344, height: 420)
        hostingView.layoutSubtreeIfNeeded()

        let fittingSize = hostingView.fittingSize
        XCTAssertGreaterThan(fittingSize.width, 300)
        XCTAssertGreaterThan(fittingSize.height, 220)

        let bitmap = hostingView.bitmapImageRepForCachingDisplay(in: hostingView.bounds)
        XCTAssertNotNil(bitmap)
        if let bitmap {
            hostingView.cacheDisplay(in: hostingView.bounds, to: bitmap)
            XCTAssertGreaterThan(bitmap.pixelsWide, 0)
            XCTAssertGreaterThan(bitmap.pixelsHigh, 0)
        }
    }

    @MainActor
    func testPopoverExposesNonColorBadgesErrorPrefixesAndAccessibilityLabels() throws {
        let fm = FileManager.default
        let tempRoot = fm.temporaryDirectory
            .appendingPathComponent("niwa-ui-a11y-\(UUID().uuidString)", isDirectory: true)
        let binDir = tempRoot.appendingPathComponent("bin", isDirectory: true)
        let niwaHome = tempRoot.appendingPathComponent("home", isDirectory: true)
        try fm.createDirectory(at: binDir, withIntermediateDirectories: true)
        try fm.createDirectory(at: niwaHome, withIntermediateDirectories: true)
        defer { try? fm.removeItem(at: tempRoot) }

        let metadata = ProductMetadata.current
        let logFile = tempRoot.appendingPathComponent("ui-a11y.log")
        let stubURL = binDir.appendingPathComponent(metadata.cliBin)
        try NiwaCLIWrapperTests.installStubBinary(at: stubURL, logFile: logFile)

        let paths = NiwaDataPaths(dataDirectory: niwaHome)
        try Data("""
        {
          "schema_version": 1,
          "glyph": ".",
          "summary": "2/8 planted - 0 blooming - resting",
          "season": "winter",
          "compact_frame": "* Niwa - Winter [Dormant] - 2/8 planted",
          "updated_at": 1752000100,
          "dormant": true,
          "planted_count": 2,
          "blooming_count": 0
        }
        """.utf8).write(to: paths.snapshotFile)

        let cli = ProcessNiwaCLI(
            environment: ["NIWA_HOME": niwaHome.path, "NIWA_BIN": stubURL.path],
            homeDirectory: tempRoot,
            metadata: metadata,
            includeStandardDirectories: false
        )
        let viewModel = MenuBarViewModel(
            paths: paths,
            cli: cli,
            idleSampler: FakeIdleSampler(nextIdleSeconds: 10.0),
            loginItemManager: FakeLoginItemManager(),
            terminalLauncher: SpyTerminalLauncher(),
            metadata: metadata,
            nowProvider: { 1_752_000_110 }
        )
        viewModel.syncFromSystem()
        viewModel.togglePauseSignals()

        XCTAssertEqual(viewModel.activeStateBadges, ["Resting", "Signals Paused"])
        XCTAssertTrue(viewModel.menuBarAccessibilityLabel.contains(metadata.displayName))
        XCTAssertTrue(viewModel.menuBarAccessibilityLabel.contains("Resting"))
        XCTAssertTrue(viewModel.menuBarAccessibilityLabel.contains("Signals Paused"))

        // Corrupt snapshot.json to verify snapshot error uses explicit non-color "Error:" prefix.
        try Data("{ invalid json".utf8).write(to: paths.snapshotFile)
        viewModel.reloadSnapshotFromDisk()
        XCTAssertEqual(viewModel.formattedSnapshotError?.hasPrefix("Error: "), true)

        let hostingView = NSHostingView(rootView: MenuBarPopoverView(viewModel: viewModel) {})
        hostingView.frame = NSRect(x: 0, y: 0, width: 344, height: 460)
        hostingView.layoutSubtreeIfNeeded()
        XCTAssertGreaterThan(hostingView.fittingSize.height, 200)
    }
}
