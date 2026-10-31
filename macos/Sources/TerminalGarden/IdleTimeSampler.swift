import CoreGraphics
import Foundation

/// Read-only sampler querying elapsed user-input idle seconds on macOS (`PLAN.md` §5, §10 T10 & T14;
/// `docs/decisions/003-macos-idle-time-source.md` & `docs/platform/idle-time.md`).
public protocol IdleTimeSampling: Sendable {
    func sampleIdleSeconds() -> Double?
}

/// Production `CoreGraphics` idle-time sampler calling `CGEventSource.secondsSinceLastEventType`
/// in-process with zero subprocess forks and zero TCC permission prompts.
public struct CoreGraphicsIdleTimeSampler: IdleTimeSampling {
    public init() {}

    public func sampleIdleSeconds() -> Double? {
        guard let anyInputEvent = CGEventType(rawValue: ~0) else {
            return nil
        }
        let seconds = CGEventSource.secondsSinceLastEventType(
            .combinedSessionState,
            eventType: anyInputEvent
        )
        guard seconds.isFinite, seconds >= 0 else {
            return nil
        }
        return seconds
    }
}
