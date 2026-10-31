import Foundation

private final class ProductMetadataBundleAnchor {}

/// Single-source product naming metadata loaded from `Info.plist` (populated by
/// `macos/Config/Product.xcconfig`, which mirrors `crates/tg-core/src/product.rs`).
public struct ProductMetadata: Sendable, Equatable {
    public let displayName: String
    public let cliBin: String
    public let cliCrate: String
    public let homebrewFormula: String
    public let homebrewCask: String
    public let bundleId: String
    public let version: String

    public init(
        displayName: String,
        cliBin: String,
        cliCrate: String,
        homebrewFormula: String,
        homebrewCask: String,
        bundleId: String,
        version: String
    ) {
        self.displayName = displayName
        self.cliBin = cliBin
        self.cliCrate = cliCrate
        self.homebrewFormula = homebrewFormula
        self.homebrewCask = homebrewCask
        self.bundleId = bundleId
        self.version = version
    }

    public static let current: ProductMetadata = load()

    public static func defaultXcconfigURL(sourceFilePath: String = #filePath) -> URL {
        URL(fileURLWithPath: sourceFilePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .appendingPathComponent("Config/Product.xcconfig")
    }

    public static func defaultBundles() -> [Bundle] {
        [.main, Bundle(for: ProductMetadataBundleAnchor.self)]
    }

    public static func load(
        bundles: [Bundle] = defaultBundles(),
        fallbackXcconfigURL: URL? = defaultXcconfigURL()
    ) -> ProductMetadata {
        for bundle in bundles {
            if let dict = bundle.infoDictionary,
               let parsed = fromDictionary(dict)
            {
                return parsed
            }
        }

        if let url = fallbackXcconfigURL,
           let raw = try? String(contentsOf: url, encoding: .utf8),
           let parsed = parseXcconfig(raw)
        {
            return parsed
        }

        preconditionFailure("Unable to resolve ProductMetadata from Info.plist or Product.xcconfig")
    }

    public static func parseXcconfig(_ raw: String) -> ProductMetadata? {
        var values: [String: String] = [:]
        for line in raw.split(whereSeparator: \.isNewline) {
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            if trimmed.isEmpty || trimmed.hasPrefix("//") {
                continue
            }
            let parts = trimmed.split(separator: "=", maxSplits: 1)
            guard parts.count == 2 else { continue }
            let key = parts[0].trimmingCharacters(in: .whitespaces)
            let value = parts[1].trimmingCharacters(in: .whitespaces)
            values[key] = value
        }
        return fromDictionary(values)
    }

    private static func fromDictionary(_ dict: [String: Any]) -> ProductMetadata? {
        func clean(_ key: String) -> String? {
            guard let raw = dict[key] as? String else { return nil }
            let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
            if trimmed.isEmpty || trimmed.contains("$(") {
                return nil
            }
            return trimmed
        }

        guard
            let displayName = clean("NIWA_DISPLAY_NAME"),
            let cliBin = clean("NIWA_CLI_BIN"),
            let cliCrate = clean("NIWA_CLI_CRATE"),
            let homebrewFormula = clean("NIWA_HOMEBREW_FORMULA"),
            let homebrewCask = clean("NIWA_HOMEBREW_CASK"),
            let bundleId = clean("NIWA_BUNDLE_ID"),
            let version = clean("NIWA_VERSION")
        else {
            return nil
        }

        return ProductMetadata(
            displayName: displayName,
            cliBin: cliBin,
            cliCrate: cliCrate,
            homebrewFormula: homebrewFormula,
            homebrewCask: homebrewCask,
            bundleId: bundleId,
            version: version
        )
    }
}
