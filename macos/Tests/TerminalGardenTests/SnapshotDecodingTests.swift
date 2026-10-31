#if canImport(XCTest)
import XCTest
#endif
import Foundation

final class SnapshotDecodingTests: XCTestCase {
    func testDecodesCanonicalSchemaV1Snapshot() throws {
        let json = """
        {
          "schema_version": 1,
          "glyph": "◈",
          "summary": "4/8 planted · 1 blooming",
          "season": "summer",
          "compact_frame": "Niwa · Summer · 4/8 planted · 1 blooming\\n  1. ◈ Moss (Blooming)",
          "updated_at": 1752000060,
          "dormant": false,
          "planted_count": 4,
          "blooming_count": 1
        }
        """
        let snapshot = try GardenSnapshot.decode(from: Data(json.utf8))
        XCTAssertEqual(snapshot.schemaVersion, 1)
        XCTAssertEqual(snapshot.glyph, "◈")
        XCTAssertEqual(snapshot.summary, "4/8 planted · 1 blooming")
        XCTAssertEqual(snapshot.season, .summer)
        XCTAssertTrue(snapshot.compactFrame.contains("1. ◈ Moss (Blooming)"))
        XCTAssertEqual(snapshot.updatedAt, 1_752_000_060)
        XCTAssertFalse(snapshot.dormant)
        XCTAssertEqual(snapshot.plantedCount, 4)
        XCTAssertEqual(snapshot.bloomingCount, 1)
    }

    func testIgnoresUnknownAdditiveFieldsUnderSchemaV1() throws {
        let json = """
        {
          "schema_version": 1,
          "glyph": "◎",
          "summary": "2/8 planted · 0 blooming · resting",
          "season": "winter",
          "compact_frame": "Niwa · Winter · resting",
          "updated_at": 1752000120,
          "dormant": true,
          "planted_count": 2,
          "blooming_count": 0,
          "future_biome": "cloud_forest",
          "future_metrics": { "sunlight": 42, "tags": ["cozy", "local"] }
        }
        """
        let snapshot = try GardenSnapshot.decode(from: Data(json.utf8))
        XCTAssertEqual(snapshot.schemaVersion, 1)
        XCTAssertEqual(snapshot.glyph, "◎")
        XCTAssertEqual(snapshot.season, .winter)
        XCTAssertTrue(snapshot.dormant)
        XCTAssertEqual(snapshot.plantedCount, 2)
        XCTAssertEqual(snapshot.bloomingCount, 0)
    }

    func testAppliesDefaultsWhenOptionalV1FieldsOmitted() throws {
        let json = """
        {
          "schema_version": 1,
          "glyph": "◌",
          "summary": "0/8 planted · 0 blooming",
          "season": "spring",
          "compact_frame": "Empty garden",
          "updated_at": 1752000000
        }
        """
        let snapshot = try GardenSnapshot.decode(from: Data(json.utf8))
        XCTAssertFalse(snapshot.dormant)
        XCTAssertEqual(snapshot.plantedCount, 0)
        XCTAssertEqual(snapshot.bloomingCount, 0)
    }

    func testRejectsInvalidAndUnsupportedFutureSchemaVersions() {
        let zeroSchema = Data("""
        {
          "schema_version": 0,
          "glyph": "◌",
          "summary": "bad",
          "season": "spring",
          "compact_frame": "bad",
          "updated_at": 1752000000
        }
        """.utf8)

        XCTAssertThrowsError(try GardenSnapshot.decode(from: zeroSchema)) { error in
            XCTAssertEqual(error as? SnapshotDecodeError, .invalidSchemaVersion(0))
        }

        let futureSchema = Data("""
        {
          "schema_version": 2,
          "glyph": "◈",
          "summary": "future",
          "season": "autumn",
          "compact_frame": "future",
          "updated_at": 1752000000
        }
        """.utf8)

        XCTAssertThrowsError(try GardenSnapshot.decode(from: futureSchema)) { error in
            XCTAssertEqual(
                error as? SnapshotDecodeError,
                .unsupportedFutureSchema(found: 2, maxSupported: 1)
            )
        }
    }

    func testFreshnessWindowClampsBackwardClockSkew() {
        let snapshot = GardenSnapshot(
            glyph: "◈",
            summary: "1/8 planted · 1 blooming",
            season: .spring,
            compactFrame: "frame",
            updatedAt: 1_752_000_000
        )
        XCTAssertTrue(snapshot.isFresh(nowUnixSeconds: 1_752_000_000))
        XCTAssertTrue(snapshot.isFresh(nowUnixSeconds: 1_752_000_059))
        XCTAssertFalse(snapshot.isFresh(nowUnixSeconds: 1_752_000_060))
        // Backward clock skew (`now < updatedAt`) clamps age to 0 so it stays fresh.
        XCTAssertTrue(snapshot.isFresh(nowUnixSeconds: 1_751_999_950))
    }

    func testSnapshotWatcherDetectsInitialCreationAndAtomicRenamesWithoutCreatingDirs() throws {
        let fm = FileManager.default
        let rootDir = fm.temporaryDirectory
            .appendingPathComponent("niwa-watcher-test-\(UUID().uuidString)", isDirectory: true)
        try fm.createDirectory(at: rootDir, withIntermediateDirectories: true)
        defer { try? fm.removeItem(at: rootDir) }

        // Start watching BEFORE `dataDirectory` exists to verify `SnapshotWatcher` never creates
        // directories on disk (`PLAN.md` §0.6: only `tg-store` writes to disk) and promotes its
        // watch once the directory and `snapshot.json` appear.
        let nestedDataDir = rootDir.appendingPathComponent("niwa-home", isDirectory: true)
        let paths = NiwaDataPaths(dataDirectory: nestedDataDir)

        let firstAppearance = expectation(description: "initial snapshot creation observed")
        let atomicReplacement = expectation(description: "atomic rename replacement observed")

        final class ObservedGlyphTracker: @unchecked Sendable {
            private let lock = NSLock()
            private var seenGlyphs: Set<String> = []
            func recordFirstTime(_ glyph: String) -> Bool {
                lock.lock()
                defer { lock.unlock() }
                return seenGlyphs.insert(glyph).inserted
            }
        }
        let tracker = ObservedGlyphTracker()

        let watcher = SnapshotWatcher(paths: paths) {
            guard FileManager.default.fileExists(atPath: paths.snapshotFile.path),
                  let bytes = try? Data(contentsOf: paths.snapshotFile),
                  let decoded = try? GardenSnapshot.decode(from: bytes)
            else {
                return
            }
            if decoded.glyph == "·", tracker.recordFirstTime("·") {
                firstAppearance.fulfill()
            } else if decoded.glyph == "◈", tracker.recordFirstTime("◈") {
                atomicReplacement.fulfill()
            }
        }
        watcher.start()
        defer { watcher.stop() }

        XCTAssertFalse(
            fm.fileExists(atPath: nestedDataDir.path),
            "SnapshotWatcher must not create dataDirectory on disk"
        )

        // Simulate `tg-store` creating `NIWA_HOME` and atomically renaming temp -> `snapshot.json`.
        try fm.createDirectory(at: nestedDataDir, withIntermediateDirectories: true)
        try Self.writeAtomically(
            glyph: "·",
            summary: "1/8 planted · 0 blooming",
            to: paths.snapshotFile
        )
        wait(for: [firstAppearance], timeout: 3.0)

        // Simulate a subsequent `tg-store` atomic temp-file `rename(2)` replacement (new inode).
        try Self.writeAtomically(
            glyph: "◈",
            summary: "1/8 planted · 1 blooming",
            to: paths.snapshotFile
        )
        wait(for: [atomicReplacement], timeout: 3.0)
    }

    private static func writeAtomically(glyph: String, summary: String, to destination: URL) throws {
        let payload = """
        {
          "schema_version": 1,
          "glyph": "\(glyph)",
          "summary": "\(summary)",
          "season": "spring",
          "compact_frame": "\(summary)",
          "updated_at": 1752000100,
          "dormant": false,
          "planted_count": 1,
          "blooming_count": \(glyph == "◈" ? 1 : 0)
        }
        """
        let tempURL = destination
            .deletingLastPathComponent()
            .appendingPathComponent(".snapshot-\(UUID().uuidString).tmp")
        try Data(payload.utf8).write(to: tempURL)
        if Darwin.rename(tempURL.path, destination.path) != 0 {
            throw NSError(domain: NSPOSIXErrorDomain, code: Int(errno))
        }
    }
}
