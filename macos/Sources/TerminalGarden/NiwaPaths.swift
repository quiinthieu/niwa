import Foundation

/// Read-only path resolver matching `tg_store::paths::NiwaPaths` on macOS (`PLAN.md` §6).
///
/// Honours `NIWA_HOME` for isolated tests and custom roots, defaulting to
/// `~/Library/Application Support/<displayName>/snapshot.json`.
public struct NiwaDataPaths: Sendable, Equatable {
    public let dataDirectory: URL

    public var snapshotFile: URL {
        dataDirectory.appendingPathComponent("snapshot.json", isDirectory: false)
    }

    public init(dataDirectory: URL) {
        self.dataDirectory = dataDirectory.standardizedFileURL
    }

    public static func resolve(
        environment: [String: String] = ProcessInfo.processInfo.environment,
        homeDirectory: URL = FileManager.default.homeDirectoryForCurrentUser,
        metadata: ProductMetadata = .current
    ) -> NiwaDataPaths {
        if let rawOverride = environment["NIWA_HOME"]?.trimmingCharacters(in: .whitespacesAndNewlines),
           !rawOverride.isEmpty
        {
            return NiwaDataPaths(dataDirectory: URL(fileURLWithPath: rawOverride, isDirectory: true))
        }

        let appSupport = homeDirectory
            .appendingPathComponent("Library", isDirectory: true)
            .appendingPathComponent("Application Support", isDirectory: true)
            .appendingPathComponent(metadata.displayName, isDirectory: true)
        return NiwaDataPaths(dataDirectory: appSupport)
    }
}
