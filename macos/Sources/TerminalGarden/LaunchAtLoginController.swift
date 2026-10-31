import Foundation
import ServiceManagement

/// User-facing registration state of `SMAppService.mainApp` (`docs/platform/macos-menubar.md` §2).
public enum LaunchAtLoginStatus: Sendable, Equatable {
    case notRegistered
    case enabled
    case requiresApproval
    case notFound

    public var isToggleOn: Bool {
        switch self {
        case .enabled, .requiresApproval:
            return true
        case .notRegistered, .notFound:
            return false
        }
    }
}

/// Testable abstraction over `ServiceManagement.SMAppService.mainApp` (`PLAN.md` §10 T14).
public protocol LoginItemManaging: Sendable {
    func currentStatus() -> LaunchAtLoginStatus
    func register() throws
    func unregister() throws
    func openSystemSettingsLoginItems()
}

/// Production `SMAppService.mainApp` adapter (macOS 13+, verified `2026-10-31`).
public struct SMAppServiceLoginItemManager: LoginItemManaging {
    public init() {}

    public func currentStatus() -> LaunchAtLoginStatus {
        switch SMAppService.mainApp.status {
        case .notRegistered:
            return .notRegistered
        case .enabled:
            return .enabled
        case .requiresApproval:
            return .requiresApproval
        case .notFound:
            return .notFound
        @unknown default:
            return .notFound
        }
    }

    public func register() throws {
        try SMAppService.mainApp.register()
    }

    public func unregister() throws {
        try SMAppService.mainApp.unregister()
    }

    public func openSystemSettingsLoginItems() {
        SMAppService.openSystemSettingsLoginItems()
    }
}
