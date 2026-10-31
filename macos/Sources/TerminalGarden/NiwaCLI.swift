import Foundation

/// Captured stdout, stderr, and exit status from invoking the `niwa` CLI.
public struct CLIExecutionResult: Sendable, Equatable {
    public let exitCode: Int32
    public let stdout: String
    public let stderr: String

    public init(exitCode: Int32, stdout: String, stderr: String) {
        self.exitCode = exitCode
        self.stdout = stdout
        self.stderr = stderr
    }
}

/// Typed errors surfaced by `NiwaCLIRunning` (`PLAN.md` §10 T14 AC 2).
public enum NiwaCLIError: Error, Sendable, Equatable, LocalizedError {
    case binaryMissing(binaryName: String, searchedPaths: [String])
    case commandFailed(arguments: [String], exitCode: Int32, stderr: String)
    case launchFailed(String)

    public var errorDescription: String? {
        switch self {
        case .binaryMissing(let binaryName, _):
            return "Could not find '\(binaryName)' in PATH or standard install locations."
        case .commandFailed(let arguments, let exitCode, let stderr):
            let detail = stderr.trimmingCharacters(in: .whitespacesAndNewlines)
            let joined = arguments.joined(separator: " ")
            if detail.isEmpty {
                return "Command '\(joined)' exited with status \(exitCode)."
            }
            return "Command '\(joined)' failed (\(exitCode)): \(detail)"
        case .launchFailed(let message):
            return "Failed to launch process: \(message)"
        }
    }
}

/// Thin-client CLI runner delegating all garden mutations and config writes to `niwa`
/// (`PLAN.md` §0.6, §3 & §6).
public protocol NiwaCLIRunning: Sendable {
    var environment: [String: String] { get }
    func locateBinary() -> URL?
    func searchedCandidatePaths() -> [String]
    @discardableResult
    func run(_ arguments: [String]) throws -> CLIExecutionResult
}

extension NiwaCLIRunning {
    @discardableResult
    public func harvestAll() throws -> CLIExecutionResult {
        try run(["harvest", "--all"])
    }

    public func getConfig(key: String) throws -> String {
        let result = try run(["config", "get", key])
        return result.stdout.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    @discardableResult
    public func setConfig(key: String, value: String) throws -> CLIExecutionResult {
        try run(["config", "set", key, value])
    }

    @discardableResult
    public func runDaemonTick(idleSeconds: Double?) throws -> CLIExecutionResult {
        if let secs = idleSeconds, secs.isFinite, secs >= 0 {
            let formatted = String(format: "%.3f", secs)
            return try run(["daemon", "--once", "--idle-secs", formatted])
        }
        return try run(["daemon", "--once"])
    }

    @discardableResult
    public func refreshStatusSnapshot() throws -> CLIExecutionResult {
        try run(["status", "--json"])
    }
}

private final class LockedDataBox: @unchecked Sendable {
    private let lock = NSLock()
    private var storage = Data()

    func store(_ data: Data) {
        lock.lock()
        storage = data
        lock.unlock()
    }

    func load() -> Data {
        lock.lock()
        defer { lock.unlock() }
        return storage
    }
}

/// Production `Foundation.Process` runner locating and invoking `niwa` without touching network
/// APIs or writing to disk (`PLAN.md` §0.4, §0.6 & §3).
public struct ProcessNiwaCLI: NiwaCLIRunning {
    public let environment: [String: String]
    public let homeDirectory: URL
    public let metadata: ProductMetadata
    public let includeStandardDirectories: Bool

    public init(
        environment: [String: String] = ProcessInfo.processInfo.environment,
        homeDirectory: URL = FileManager.default.homeDirectoryForCurrentUser,
        metadata: ProductMetadata = .current,
        includeStandardDirectories: Bool = true
    ) {
        self.environment = environment
        self.homeDirectory = homeDirectory
        self.metadata = metadata
        self.includeStandardDirectories = includeStandardDirectories
    }

    public func searchedCandidatePaths() -> [String] {
        var candidates: [String] = []
        let binName = metadata.cliBin

        if let explicit = environment["NIWA_BIN"]?.trimmingCharacters(in: .whitespacesAndNewlines),
           !explicit.isEmpty
        {
            candidates.append(explicit)
        }

        if includeStandardDirectories {
            let standardDirs = [
                "/opt/homebrew/bin",
                "/usr/local/bin",
                homeDirectory.appendingPathComponent(".cargo/bin", isDirectory: true).path,
            ]
            for dir in standardDirs {
                let path = URL(fileURLWithPath: dir, isDirectory: true)
                    .appendingPathComponent(binName, isDirectory: false)
                    .path
                if !candidates.contains(path) {
                    candidates.append(path)
                }
            }
        }

        if let rawPath = environment["PATH"], !rawPath.isEmpty {
            for segment in rawPath.split(separator: ":") {
                let dir = String(segment).trimmingCharacters(in: .whitespacesAndNewlines)
                guard !dir.isEmpty else { continue }
                let path = URL(fileURLWithPath: dir, isDirectory: true)
                    .appendingPathComponent(binName, isDirectory: false)
                    .path
                if !candidates.contains(path) {
                    candidates.append(path)
                }
            }
        }

        return candidates
    }

    public func locateBinary() -> URL? {
        let fm = FileManager.default
        for candidate in searchedCandidatePaths() {
            var isDir: ObjCBool = false
            if fm.fileExists(atPath: candidate, isDirectory: &isDir),
               !isDir.boolValue,
               fm.isExecutableFile(atPath: candidate)
            {
                return URL(fileURLWithPath: candidate, isDirectory: false)
            }
        }
        return nil
    }

    @discardableResult
    public func run(_ arguments: [String]) throws -> CLIExecutionResult {
        guard let binaryURL = locateBinary() else {
            throw NiwaCLIError.binaryMissing(
                binaryName: metadata.cliBin,
                searchedPaths: searchedCandidatePaths()
            )
        }

        let process = Process()
        process.executableURL = binaryURL
        process.arguments = arguments
        process.environment = environment

        let stdoutPipe = Pipe()
        let stderrPipe = Pipe()
        process.standardOutput = stdoutPipe
        process.standardError = stderrPipe
        process.standardInput = FileHandle.nullDevice

        do {
            try process.run()
        } catch {
            throw NiwaCLIError.launchFailed(error.localizedDescription)
        }

        let stderrBox = LockedDataBox()
        let group = DispatchGroup()
        group.enter()
        let stderrReader = stderrPipe.fileHandleForReading
        DispatchQueue.global(qos: .userInitiated).async {
            stderrBox.store(stderrReader.readDataToEndOfFile())
            group.leave()
        }

        let stdoutData = stdoutPipe.fileHandleForReading.readDataToEndOfFile()
        group.wait()
        process.waitUntilExit()

        let stdout = String(decoding: stdoutData, as: UTF8.self)
        let stderr = String(decoding: stderrBox.load(), as: UTF8.self)
        let status = process.terminationStatus

        if status != 0 {
            throw NiwaCLIError.commandFailed(
                arguments: arguments,
                exitCode: status,
                stderr: stderr
            )
        }

        return CLIExecutionResult(exitCode: status, stdout: stdout, stderr: stderr)
    }
}

/// Launches the interactive `niwa` TUI inside macOS `Terminal.app` without writing temporary
/// `.command` files or triggering AppleEvents TCC prompts (`docs/platform/macos-menubar.md` §4).
public protocol TerminalLaunching: Sendable {
    func openInteractiveTUI(binaryURL: URL) throws
}

public struct ProcessTerminalLauncher: TerminalLaunching {
    public init() {}

    public func openInteractiveTUI(binaryURL: URL) throws {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/open", isDirectory: false)
        process.arguments = ["-a", "Terminal", binaryURL.path]
        process.standardInput = FileHandle.nullDevice
        process.standardOutput = FileHandle.nullDevice

        let stderrPipe = Pipe()
        process.standardError = stderrPipe

        do {
            try process.run()
        } catch {
            throw NiwaCLIError.launchFailed(error.localizedDescription)
        }

        let stderrData = stderrPipe.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()
        if process.terminationStatus != 0 {
            let stderr = String(decoding: stderrData, as: UTF8.self)
            throw NiwaCLIError.commandFailed(
                arguments: ["open", "-a", "Terminal", binaryURL.path],
                exitCode: process.terminationStatus,
                stderr: stderr
            )
        }
    }
}
