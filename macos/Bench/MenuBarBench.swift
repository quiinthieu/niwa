import AppKit
import Darwin
import Foundation
import SwiftUI

struct BenchIdleSampler: IdleTimeSampling {
    func sampleIdleSeconds() -> Double? { 12.0 }
}

struct BenchLoginItemManager: LoginItemManaging {
    func currentStatus() -> LaunchAtLoginStatus { .notRegistered }
    func register() throws {}
    func unregister() throws {}
    func openSystemSettingsLoginItems() {}
}

struct BenchTerminalLauncher: TerminalLaunching {
    func openInteractiveTUI(binaryURL: URL) throws {}
}

final class CountingCLIRunner: NiwaCLIRunning, @unchecked Sendable {
    private let inner: ProcessNiwaCLI
    private let lock = NSLock()
    private var invocationCount: Int = 0

    init(inner: ProcessNiwaCLI) {
        self.inner = inner
    }

    var environment: [String: String] { inner.environment }

    func locateBinary() -> URL? { inner.locateBinary() }

    func searchedCandidatePaths() -> [String] { inner.searchedCandidatePaths() }

    @discardableResult
    func run(_ arguments: [String]) throws -> CLIExecutionResult {
        lock.lock()
        invocationCount += 1
        lock.unlock()
        return try inner.run(arguments)
    }

    func count() -> Int {
        lock.lock()
        defer { lock.unlock() }
        return invocationCount
    }
}

struct MacosMenuBarReport: Codable {
    let timer_interval_secs: Double
    let timer_tolerance_secs: Double
    let observation_window_ms: Double
    let user_cpu_ms: Double
    let sys_cpu_ms: Double
    let total_cpu_ms: Double
    let idle_cpu_pct: Double
    let idle_spawns_between_ticks: Int
    let phys_footprint_mib: Double
    let snapshot_reload_p50_us: Double
    let snapshot_reload_p95_us: Double
    let snapshot_reload_max_us: Double
}

private func timevalToMs(_ tv: timeval) -> Double {
    (Double(tv.tv_sec) * 1_000.0) + (Double(tv.tv_usec) / 1_000.0)
}

private func currentPhysFootprintMiB() -> Double {
    var info = task_vm_info_data_t()
    var count = mach_msg_type_number_t(MemoryLayout<task_vm_info_data_t>.size / MemoryLayout<natural_t>.size)
    let kr = withUnsafeMutablePointer(to: &info) { infoPtr in
        infoPtr.withMemoryRebound(to: integer_t.self, capacity: Int(count)) { intPtr in
            task_info(mach_task_self_, task_flavor_t(TASK_VM_INFO), intPtr, &count)
        }
    }
    guard kr == KERN_SUCCESS else { return 0.0 }
    return Double(info.phys_footprint) / (1_024.0 * 1_024.0)
}

private func round3(_ v: Double) -> Double {
    (v * 1_000.0).rounded() / 1_000.0
}

private func round4(_ v: Double) -> Double {
    (v * 10_000.0).rounded() / 10_000.0
}

private func installStubNiwaIfNeeded(at stubURL: URL) throws {
    let script = """
    #!/bin/sh
    if [ "$1" = "config" ] && [ "$2" = "get" ]; then
      case "$3" in
        signals.paused) echo "false" ;;
        signals.activity.enabled) echo "false" ;;
        signals.git.enabled) echo "false" ;;
        signals.shell.enabled) echo "false" ;;
        menubar.show_title) echo "true" ;;
        *) echo "none" ;;
      esac
      exit 0
    fi
    exit 0
    """
    try Data(script.utf8).write(to: stubURL)
    try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: stubURL.path)
}

@main
struct MenuBarBench {
    @MainActor
    static func main() throws {
        _ = NSApplication.shared

        let fm = FileManager.default
        let tempRoot = fm.temporaryDirectory
            .appendingPathComponent("niwa-menubar-bench-\(UUID().uuidString)", isDirectory: true)
        let niwaHome = tempRoot.appendingPathComponent("home", isDirectory: true)
        let binDir = tempRoot.appendingPathComponent("bin", isDirectory: true)
        try fm.createDirectory(at: niwaHome, withIntermediateDirectories: true)
        try fm.createDirectory(at: binDir, withIntermediateDirectories: true)
        defer { try? fm.removeItem(at: tempRoot) }

        let metadata = ProductMetadata.current
        let binPath: String
        if let provided = ProcessInfo.processInfo.environment["NIWA_BENCH_BIN"],
           fm.isExecutableFile(atPath: provided)
        {
            binPath = provided
        } else {
            let stub = binDir.appendingPathComponent(metadata.cliBin)
            try installStubNiwaIfNeeded(at: stub)
            binPath = stub.path
        }

        let paths = NiwaDataPaths(dataDirectory: niwaHome)
        let nowSecs = Int64(Date().timeIntervalSince1970)
        let snapshotJSON = """
        {
          "schema_version": 1,
          "glyph": "◈",
          "summary": "5/9 planted · 2 blooming",
          "season": "spring",
          "compact_frame": " ◈ Niwa · Spring · 5/9 planted · 2 blooming\\n  #1 ░◈░ [Blm◈] Sakura Bonsai\\n  #2 ├│┤ [Matr] Meadow Daisy",
          "updated_at": \(nowSecs),
          "dormant": false,
          "planted_count": 5,
          "blooming_count": 2
        }
        """
        try Data(snapshotJSON.utf8).write(to: paths.snapshotFile)

        let innerCLI = ProcessNiwaCLI(
            environment: ["NIWA_HOME": niwaHome.path, "HOME": tempRoot.path, "NIWA_BIN": binPath],
            homeDirectory: tempRoot,
            metadata: metadata,
            includeStandardDirectories: false
        )
        let countingCLI = CountingCLIRunner(inner: innerCLI)

        let viewModel = MenuBarViewModel(
            paths: paths,
            cli: countingCLI,
            idleSampler: BenchIdleSampler(),
            loginItemManager: BenchLoginItemManager(),
            terminalLauncher: BenchTerminalLauncher(),
            metadata: metadata
        )
        viewModel.start()

        let hostingView = NSHostingView(rootView: MenuBarPopoverView(viewModel: viewModel) {})
        hostingView.frame = NSRect(x: 0, y: 0, width: 344, height: 420)
        hostingView.layoutSubtreeIfNeeded()

        // Drain startup tasks for 250 ms before measuring steady-state idle behavior.
        let warmupEnd = Date().addingTimeInterval(0.25)
        while Date() < warmupEnd {
            _ = RunLoop.main.run(mode: .default, before: warmupEnd)
        }

        let idleWindowSecs = Double(ProcessInfo.processInfo.environment["NIWA_MACOS_BENCH_IDLE_SECS"] ?? "") ?? 5.0
        let spawnsBefore = countingCLI.count()
        var usageStart = rusage()
        getrusage(RUSAGE_SELF, &usageStart)
        let wallStart = DispatchTime.now().uptimeNanoseconds

        let deadline = Date().addingTimeInterval(idleWindowSecs)
        while Date() < deadline {
            _ = RunLoop.main.run(mode: .default, before: deadline)
        }

        let wallEnd = DispatchTime.now().uptimeNanoseconds
        var usageEnd = rusage()
        getrusage(RUSAGE_SELF, &usageEnd)
        let spawnsAfter = countingCLI.count()

        let wallMs = Double(wallEnd - wallStart) / 1_000_000.0
        let userCpuMs = max(0.0, timevalToMs(usageEnd.ru_utime) - timevalToMs(usageStart.ru_utime))
        let sysCpuMs = max(0.0, timevalToMs(usageEnd.ru_stime) - timevalToMs(usageStart.ru_stime))
        let totalCpuMs = userCpuMs + sysCpuMs
        let idleCpuPct = wallMs > 0 ? (totalCpuMs / wallMs) * 100.0 : 0.0
        let idleSpawns = spawnsAfter - spawnsBefore
        let footprintMiB = currentPhysFootprintMiB()

        // Measure 200 consecutive `snapshot.json` read + decode cycles (`reloadSnapshotFromDisk`).
        for _ in 0..<10 {
            viewModel.reloadSnapshotFromDisk()
        }
        var reloadSamplesUs: [Double] = []
        reloadSamplesUs.reserveCapacity(200)
        for _ in 0..<200 {
            let t0 = DispatchTime.now().uptimeNanoseconds
            viewModel.reloadSnapshotFromDisk()
            let elapsedUs = Double(DispatchTime.now().uptimeNanoseconds - t0) / 1_000.0
            reloadSamplesUs.append(elapsedUs)
        }
        reloadSamplesUs.sort()
        let p50Idx = max(0, ((200 * 50 + 99) / 100) - 1)
        let p95Idx = max(0, ((200 * 95 + 99) / 100) - 1)

        viewModel.stop()

        let report = MacosMenuBarReport(
            timer_interval_secs: MenuBarViewModel.tickIntervalSeconds,
            timer_tolerance_secs: MenuBarViewModel.tickToleranceSeconds,
            observation_window_ms: round3(wallMs),
            user_cpu_ms: round3(userCpuMs),
            sys_cpu_ms: round3(sysCpuMs),
            total_cpu_ms: round3(totalCpuMs),
            idle_cpu_pct: round4(idleCpuPct),
            idle_spawns_between_ticks: idleSpawns,
            phys_footprint_mib: round3(footprintMiB),
            snapshot_reload_p50_us: round3(reloadSamplesUs[p50Idx]),
            snapshot_reload_p95_us: round3(reloadSamplesUs[p95Idx]),
            snapshot_reload_max_us: round3(reloadSamplesUs[reloadSamplesUs.count - 1])
        )

        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        let jsonData = try encoder.encode(report)
        if let outPath = ProcessInfo.processInfo.environment["NIWA_MACOS_BENCH_JSON_OUT"] {
            try jsonData.write(to: URL(fileURLWithPath: outPath))
        } else if let str = String(data: jsonData, encoding: .utf8) {
            print(str)
        }
    }
}
