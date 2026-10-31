import Foundation

private final class ConsentCatalogBundleAnchor {}

/// Opt-in work signal identifiers matching Rust `tg_store::config::SignalKind` (`PLAN.md` §5 & §6).
public enum SignalKindName: String, CaseIterable, Sendable, Equatable, Identifiable {
    case activity
    case git
    case shell

    public var id: String { rawValue }
}

/// Disclosure describing Niwa's always-on wall-clock baseline (`[baseline.time]` in `data/consent.toml`).
public struct BaselineDisclosure: Sendable, Equatable {
    public let displayName: String
    public let summary: String
    public let source: String
    public let stores: [String]
    public let neverStores: [String]

    public init(
        displayName: String,
        summary: String,
        source: String,
        stores: [String],
        neverStores: [String]
    ) {
        self.displayName = displayName
        self.summary = summary
        self.source = source
        self.stores = stores
        self.neverStores = neverStores
    }
}

/// Shared opt-in signal consent disclosure loaded from `data/consent.toml` (`PLAN.md` §5 & §10 T15).
public struct SignalDisclosure: Sendable, Equatable {
    public let displayName: String
    public let summary: String
    public let source: String
    public let stores: [String]
    public let neverStores: [String]
    public let prompt: String

    public init(
        displayName: String,
        summary: String,
        source: String,
        stores: [String],
        neverStores: [String],
        prompt: String
    ) {
        self.displayName = displayName
        self.summary = summary
        self.source = source
        self.stores = stores
        self.neverStores = neverStores
        self.prompt = prompt
    }

    /// Renders the shared multi-line disclosure body matching Rust
    /// `tg_signals::consent::SignalDisclosure::render_disclosure_body` byte for byte.
    public func renderDisclosureBody(signal: SignalKindName) -> String {
        var lines: [String] = []
        lines.append("Signal: \(signal.rawValue) (\(displayName))")
        lines.append("  \(summary)")
        lines.append("  Source: \(source)")
        lines.append("")
        lines.append("What Niwa will store locally:")
        for item in stores {
            lines.append("  - \(item)")
        }
        lines.append("")
        lines.append("What Niwa NEVER stores or transmits:")
        for item in neverStores {
            lines.append("  - \(item)")
        }
        lines.append("")
        lines.append(prompt)
        return lines.joined(separator: "\n")
    }

    /// Renders the full CLI consent prompt matching `niwa signals enable <name>` byte for byte.
    public func renderDisclosureText(signal: SignalKindName) -> String {
        "\(renderDisclosureBody(signal: signal)) [y/N]: "
    }
}

public enum ConsentCatalogError: Error, Sendable, Equatable, LocalizedError {
    case syntaxError(line: Int, detail: String)
    case missingSection(String)
    case emptyField(section: String, field: String)

    public var errorDescription: String? {
        switch self {
        case .syntaxError(let line, let detail):
            return "Invalid consent.toml at line \(line): \(detail)"
        case .missingSection(let section):
            return "Missing section '[\(section)]' in consent.toml"
        case .emptyField(let section, let field):
            return "Consent disclosure field '\(field)' in '[\(section)]' must not be empty"
        }
    }
}

/// Validated catalog of shared signal disclosures loaded from bundled `consent.toml`
/// (`data/consent.toml`) with zero duplicated consent strings (`PLAN.md` §10 T15 AC 2).
public struct ConsentCatalog: Sendable, Equatable {
    public let baselineTime: BaselineDisclosure
    private let activity: SignalDisclosure
    private let git: SignalDisclosure
    private let shell: SignalDisclosure

    public init(
        baselineTime: BaselineDisclosure,
        activity: SignalDisclosure,
        git: SignalDisclosure,
        shell: SignalDisclosure
    ) {
        self.baselineTime = baselineTime
        self.activity = activity
        self.git = git
        self.shell = shell
    }

    public static let current: ConsentCatalog = load()

    public static func defaultBundles() -> [Bundle] {
        [.main, Bundle(for: ConsentCatalogBundleAnchor.self)]
    }

    public static func defaultConsentTomlURL(sourceFilePath: String = #filePath) -> URL {
        URL(fileURLWithPath: sourceFilePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .appendingPathComponent("data/consent.toml")
    }

    public static func load(
        bundles: [Bundle] = defaultBundles(),
        fallbackURL: URL? = defaultConsentTomlURL()
    ) -> ConsentCatalog {
        for bundle in bundles {
            if let url = bundle.url(forResource: "consent", withExtension: "toml"),
               let raw = try? String(contentsOf: url, encoding: .utf8),
               let parsed = try? parseToml(raw)
            {
                return parsed
            }
        }

        if let url = fallbackURL,
           let raw = try? String(contentsOf: url, encoding: .utf8),
           let parsed = try? parseToml(raw)
        {
            return parsed
        }

        preconditionFailure("Unable to load shared data/consent.toml from bundle resources or repository path")
    }

    public func signal(_ kind: SignalKindName) -> SignalDisclosure {
        switch kind {
        case .activity:
            return activity
        case .git:
            return git
        case .shell:
            return shell
        }
    }

    /// Parses the constrained TOML schema used by `data/consent.toml` (sections, quoted strings,
    /// and string arrays) without third-party Swift dependencies (`PLAN.md` §3.3).
    public static func parseToml(_ raw: String) throws -> ConsentCatalog {
        enum Value {
            case string(String)
            case array([String])
        }

        var sections: [String: [String: Value]] = [:]
        var currentSection: String?
        var pendingArray: (section: String, key: String, items: [String])?

        let rawLines = raw.components(separatedBy: .newlines)
        for (idx, rawLine) in rawLines.enumerated() {
            let lineNum = idx + 1
            let trimmed = stripCommentOutsideQuotes(rawLine).trimmingCharacters(in: .whitespaces)
            if trimmed.isEmpty {
                continue
            }

            if var active = pendingArray {
                if trimmed == "]" {
                    sections[active.section, default: [:]][active.key] = .array(active.items)
                    pendingArray = nil
                    continue
                }
                let (elements, closed) = try parseArrayElementsLine(trimmed, lineNum: lineNum)
                active.items.append(contentsOf: elements)
                if closed {
                    sections[active.section, default: [:]][active.key] = .array(active.items)
                    pendingArray = nil
                } else {
                    pendingArray = active
                }
                continue
            }

            if trimmed.hasPrefix("[") && trimmed.hasSuffix("]") {
                let name = String(trimmed.dropFirst().dropLast()).trimmingCharacters(in: .whitespaces)
                guard !name.isEmpty else {
                    throw ConsentCatalogError.syntaxError(line: lineNum, detail: "empty section header")
                }
                currentSection = name
                if sections[name] == nil {
                    sections[name] = [:]
                }
                continue
            }

            guard let section = currentSection else {
                throw ConsentCatalogError.syntaxError(line: lineNum, detail: "key-value outside section")
            }
            guard let eqIndex = trimmed.firstIndex(of: "=") else {
                throw ConsentCatalogError.syntaxError(line: lineNum, detail: "expected '='")
            }

            let key = trimmed[..<eqIndex].trimmingCharacters(in: .whitespaces)
            let rhs = trimmed[trimmed.index(after: eqIndex)...].trimmingCharacters(in: .whitespaces)
            guard !key.isEmpty else {
                throw ConsentCatalogError.syntaxError(line: lineNum, detail: "empty key")
            }

            if rhs.hasPrefix("\"") {
                let strVal = try parseQuotedString(rhs, lineNum: lineNum)
                sections[section, default: [:]][key] = .string(strVal)
            } else if rhs.hasPrefix("[") {
                let afterBracket = String(rhs.dropFirst()).trimmingCharacters(in: .whitespaces)
                if afterBracket.isEmpty {
                    pendingArray = (section: section, key: key, items: [])
                } else if afterBracket == "]" {
                    sections[section, default: [:]][key] = .array([])
                } else {
                    let (elements, closed) = try parseArrayElementsLine(afterBracket, lineNum: lineNum)
                    if closed {
                        sections[section, default: [:]][key] = .array(elements)
                    } else {
                        pendingArray = (section: section, key: key, items: elements)
                    }
                }
            } else {
                throw ConsentCatalogError.syntaxError(
                    line: lineNum,
                    detail: "unsupported TOML value '\(rhs)'"
                )
            }
        }

        if let unclosed = pendingArray {
            throw ConsentCatalogError.syntaxError(
                line: rawLines.count,
                detail: "unclosed array for '\(unclosed.section).\(unclosed.key)'"
            )
        }

        func requireString(_ sectionName: String, _ table: [String: Value], _ field: String) throws -> String {
            guard case .string(let val)? = table[field],
                  !val.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            else {
                throw ConsentCatalogError.emptyField(section: sectionName, field: field)
            }
            return val
        }

        func requireArray(_ sectionName: String, _ table: [String: Value], _ field: String) throws -> [String] {
            guard case .array(let items)? = table[field],
                  !items.isEmpty,
                  items.allSatisfy({ !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty })
            else {
                throw ConsentCatalogError.emptyField(section: sectionName, field: field)
            }
            return items
        }

        guard let timeTable = sections["baseline.time"] else {
            throw ConsentCatalogError.missingSection("baseline.time")
        }
        let baseline = BaselineDisclosure(
            displayName: try requireString("baseline.time", timeTable, "display_name"),
            summary: try requireString("baseline.time", timeTable, "summary"),
            source: try requireString("baseline.time", timeTable, "source"),
            stores: try requireArray("baseline.time", timeTable, "stores"),
            neverStores: try requireArray("baseline.time", timeTable, "never_stores")
        )

        func buildSignal(_ sectionName: String) throws -> SignalDisclosure {
            guard let table = sections[sectionName] else {
                throw ConsentCatalogError.missingSection(sectionName)
            }
            return SignalDisclosure(
                displayName: try requireString(sectionName, table, "display_name"),
                summary: try requireString(sectionName, table, "summary"),
                source: try requireString(sectionName, table, "source"),
                stores: try requireArray(sectionName, table, "stores"),
                neverStores: try requireArray(sectionName, table, "never_stores"),
                prompt: try requireString(sectionName, table, "prompt")
            )
        }

        return ConsentCatalog(
            baselineTime: baseline,
            activity: try buildSignal("signals.activity"),
            git: try buildSignal("signals.git"),
            shell: try buildSignal("signals.shell")
        )
    }

    private static func stripCommentOutsideQuotes(_ line: String) -> String {
        var inQuotes = false
        var escaped = false
        var out = ""
        for ch in line {
            if escaped {
                out.append(ch)
                escaped = false
                continue
            }
            if ch == "\\" && inQuotes {
                out.append(ch)
                escaped = true
                continue
            }
            if ch == "\"" {
                inQuotes.toggle()
                out.append(ch)
                continue
            }
            if ch == "#" && !inQuotes {
                break
            }
            out.append(ch)
        }
        return out
    }

    private static func parseQuotedString(_ text: String, lineNum: Int) throws -> String {
        let trimmed = text.trimmingCharacters(in: .whitespaces)
        guard trimmed.first == "\"" else {
            throw ConsentCatalogError.syntaxError(line: lineNum, detail: "expected opening quote")
        }
        var out = ""
        var escaped = false
        var idx = trimmed.index(after: trimmed.startIndex)
        while idx < trimmed.endIndex {
            let ch = trimmed[idx]
            if escaped {
                switch ch {
                case "\"": out.append("\"")
                case "\\": out.append("\\")
                case "n": out.append("\n")
                case "t": out.append("\t")
                default: out.append(ch)
                }
                escaped = false
                idx = trimmed.index(after: idx)
                continue
            }
            if ch == "\\" {
                escaped = true
                idx = trimmed.index(after: idx)
                continue
            }
            if ch == "\"" {
                let trailing = trimmed[trimmed.index(after: idx)...].trimmingCharacters(in: .whitespaces)
                if !trailing.isEmpty {
                    throw ConsentCatalogError.syntaxError(
                        line: lineNum,
                        detail: "unexpected trailing tokens '\(trailing)'"
                    )
                }
                return out
            }
            out.append(ch)
            idx = trimmed.index(after: idx)
        }
        throw ConsentCatalogError.syntaxError(line: lineNum, detail: "unterminated string literal")
    }

    private static func parseArrayElementsLine(
        _ line: String,
        lineNum: Int
    ) throws -> (elements: [String], closed: Bool) {
        var working = line.trimmingCharacters(in: .whitespaces)
        var closed = false
        if working.hasSuffix("]") {
            closed = true
            working = String(working.dropLast()).trimmingCharacters(in: .whitespaces)
        }
        if working.isEmpty {
            return ([], closed)
        }

        var items: [String] = []
        var cursor = working.startIndex
        while cursor < working.endIndex {
            while cursor < working.endIndex && (working[cursor].isWhitespace || working[cursor] == ",") {
                cursor = working.index(after: cursor)
            }
            if cursor >= working.endIndex {
                break
            }
            guard working[cursor] == "\"" else {
                throw ConsentCatalogError.syntaxError(
                    line: lineNum,
                    detail: "expected quoted array element"
                )
            }
            var item = ""
            var escaped = false
            cursor = working.index(after: cursor)
            var foundClose = false
            while cursor < working.endIndex {
                let ch = working[cursor]
                if escaped {
                    switch ch {
                    case "\"": item.append("\"")
                    case "\\": item.append("\\")
                    case "n": item.append("\n")
                    case "t": item.append("\t")
                    default: item.append(ch)
                    }
                    escaped = false
                    cursor = working.index(after: cursor)
                    continue
                }
                if ch == "\\" {
                    escaped = true
                    cursor = working.index(after: cursor)
                    continue
                }
                if ch == "\"" {
                    foundClose = true
                    cursor = working.index(after: cursor)
                    break
                }
                item.append(ch)
                cursor = working.index(after: cursor)
            }
            guard foundClose else {
                throw ConsentCatalogError.syntaxError(
                    line: lineNum,
                    detail: "unterminated array string literal"
                )
            }
            items.append(item)
        }
        return (items, closed)
    }
}
