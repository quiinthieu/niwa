import Foundation

/// Active garden season serialized in `snapshot.json` (`docs/snapshot-schema.md` §4).
public enum GardenSeason: String, Codable, Sendable, Equatable, CaseIterable {
    case spring
    case summer
    case autumn
    case winter

    public var displayName: String {
        switch self {
        case .spring: return "Spring"
        case .summer: return "Summer"
        case .autumn: return "Autumn"
        case .winter: return "Winter"
        }
    }
}

/// Errors surfaced when decoding `snapshot.json` (`docs/snapshot-schema.md` §2).
public enum SnapshotDecodeError: Error, Sendable, Equatable, LocalizedError {
    case invalidJSON(String)
    case invalidSchemaVersion(Int)
    case unsupportedFutureSchema(found: Int, maxSupported: Int)

    public var errorDescription: String? {
        switch self {
        case .invalidJSON(let detail):
            return "Invalid snapshot.json payload: \(detail)"
        case .invalidSchemaVersion(let version):
            return "Invalid snapshot.json schema_version \(version) (expected >= 1)"
        case .unsupportedFutureSchema(let found, let maxSupported):
            return "snapshot.json uses schema_version \(found), newer than supported version \(maxSupported)"
        }
    }
}

/// Read-only Swift projection of `snapshot.json` (`schema_version = 1`, `PLAN.md` §3 & §6).
///
/// Tolerates additive unknown JSON keys under `schema_version = 1` and defaults optional v1
/// fields (`dormant`, `planted_count`, `blooming_count`) so minor Rust schema additions never
/// break older menu bar builds (`PLAN.md` §9, §10 T13–T14 & `docs/snapshot-schema.md` §2).
public struct GardenSnapshot: Codable, Sendable, Equatable {
    public static let currentSchemaVersion: Int = 1
    public static let maxFreshnessAgeSeconds: Int64 = 60

    public let schemaVersion: Int
    public let glyph: String
    public let summary: String
    public let season: GardenSeason
    public let compactFrame: String
    public let updatedAt: Int64
    public let dormant: Bool
    public let plantedCount: Int
    public let bloomingCount: Int

    public init(
        schemaVersion: Int = GardenSnapshot.currentSchemaVersion,
        glyph: String,
        summary: String,
        season: GardenSeason,
        compactFrame: String,
        updatedAt: Int64,
        dormant: Bool = false,
        plantedCount: Int = 0,
        bloomingCount: Int = 0
    ) {
        self.schemaVersion = schemaVersion
        self.glyph = glyph
        self.summary = summary
        self.season = season
        self.compactFrame = compactFrame
        self.updatedAt = updatedAt
        self.dormant = dormant
        self.plantedCount = plantedCount
        self.bloomingCount = bloomingCount
    }

    enum CodingKeys: String, CodingKey {
        case schemaVersion = "schema_version"
        case glyph
        case summary
        case season
        case compactFrame = "compact_frame"
        case updatedAt = "updated_at"
        case dormant
        case plantedCount = "planted_count"
        case bloomingCount = "blooming_count"
    }

    public init(from decoder: any Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.schemaVersion = try container.decode(Int.self, forKey: .schemaVersion)
        self.glyph = try container.decode(String.self, forKey: .glyph)
        self.summary = try container.decode(String.self, forKey: .summary)
        self.season = try container.decode(GardenSeason.self, forKey: .season)
        self.compactFrame = try container.decode(String.self, forKey: .compactFrame)
        self.updatedAt = try container.decode(Int64.self, forKey: .updatedAt)
        self.dormant = try container.decodeIfPresent(Bool.self, forKey: .dormant) ?? false
        self.plantedCount = try container.decodeIfPresent(Int.self, forKey: .plantedCount) ?? 0
        self.bloomingCount = try container.decodeIfPresent(Int.self, forKey: .bloomingCount) ?? 0
    }

    private struct SchemaHeader: Decodable {
        let schema_version: Int
    }

    /// Decodes a `GardenSnapshot` from raw UTF-8 JSON bytes, validating `schema_version` first.
    public static func decode(from data: Data) throws -> GardenSnapshot {
        let decoder = JSONDecoder()
        let header: SchemaHeader
        do {
            header = try decoder.decode(SchemaHeader.self, from: data)
        } catch {
            throw SnapshotDecodeError.invalidJSON(error.localizedDescription)
        }

        if header.schema_version <= 0 {
            throw SnapshotDecodeError.invalidSchemaVersion(header.schema_version)
        }
        if header.schema_version > currentSchemaVersion {
            throw SnapshotDecodeError.unsupportedFutureSchema(
                found: header.schema_version,
                maxSupported: currentSchemaVersion
            )
        }

        do {
            return try decoder.decode(GardenSnapshot.self, from: data)
        } catch {
            throw SnapshotDecodeError.invalidJSON(error.localizedDescription)
        }
    }

    /// Returns `true` when `snapshot.json` was written fewer than 60 seconds ago
    /// (clamping negative clock skew to `0`, matching `tg_store::snapshot::Snapshot::is_fresh`).
    public func isFresh(nowUnixSeconds: Int64) -> Bool {
        let age = max(Int64(0), nowUnixSeconds - updatedAt)
        return age < Self.maxFreshnessAgeSeconds
    }
}
