# ADR 003: macOS Idle-Time Sampling Strategy (`CoreGraphics` in Swift + Safe `ioreg` Fallback in Rust)

- **Status:** Accepted
- **Date:** 2026-10-31
- **Context:** `PLAN.md` §3.1, §3.3, §5, §7, §10 T10 ("macOS idle time (an ADR decides between a Rust binding and the Swift app writing `activity` events)") & T14 ("If T10 chose the Swift route, it also hosts the idle-time sampler")

## Context

When the opt-in `activity` signal is enabled (`niwa signals enable activity`), Niwa samples system user-input idle duration once per 60-second garden tick (`PLAN.md` §5 & §7). It records only `{"ts": <i64>, "kind": "activity", "active": <bool>}` in `events.jsonl`—never keystrokes, cursor coordinates, active apps, or window titles.

On macOS, `PLAN.md` §5 & §10 T10 ask for an explicit ADR choosing between:
1. A Rust binding to `CGEventSourceSecondsSinceLastEventType` in `CoreGraphics.framework`, or
2. Having the native Swift 6 menu bar app (`macos/TerminalGarden`, T14) sample `CoreGraphics` in-process and append `activity` events to `events.jsonl`.

Three hard workspace constraints shape this decision:
- Root `Cargo.toml` enforces `[workspace.lints.rust] unsafe_code = "forbid"` across every Rust crate.
- `PLAN.md` §3.3 restricts Rust dependencies to an approved list (excluding `core-graphics`, `objc2`, or raw `libc` bindings).
- `PLAN.md` §1.3 & §3 establish Niwa as **terminal-first**, with the macOS menu bar cask (`niwa-garden`) as an optional companion alongside the standalone CLI formula (`niwa`).

## Options Considered

### Option A: Raw Rust C-FFI or Third-Party Crate Binding to `CoreGraphics`
- **Mechanism:** Link `CoreGraphics.framework` in `tg-signals` via `extern "C" { fn CGEventSourceSecondsSinceLastEventType(state_id: u32, event_type: u32) -> f64; }` or add `core-graphics` to `Cargo.toml`.
- **Pros:** Direct C call inside `niwa daemon` with sub-microsecond overhead.
- **Cons:** Violates `unsafe_code = "forbid"` in `Cargo.toml` (or requires expanding `PLAN.md` §3.3's dependency budget for a single scalar read called once every 60 seconds).

### Option B: Swift Menu Bar App Exclusively Writes `activity` Events
- **Mechanism:** `Niwa.app` (`macos/TerminalGarden`, T14) calls `CGEventSource.secondsSinceLastEventType(.combinedSessionState, eventType: CGEventType(rawValue: ~0)!)` on its existing 60-second timer (`PLAN.md` §7) and appends `activity` events; `niwa daemon` performs no idle sampling on macOS.
- **Pros:** Native in-process `CoreGraphics` call in Swift with zero `unsafe` Rust and zero extra crates.
- **Cons:** CLI-only macOS users who install `brew install quiinthieu/tap/niwa` without the `niwa-garden` menu bar app would receive no `activity` sampling even after enabling `signals.activity` and running `niwa daemon`.

### Option C (Chosen): Swift `CoreGraphics` Sampler in `Niwa.app` (T14) + Safe-Rust `/usr/sbin/ioreg` Fallback in `niwa daemon` (T10)
- **Mechanism:**
  1. **Native `CoreGraphics` route when `Niwa.app` is running (T14):** `macos/TerminalGarden` hosts the in-process `CGEventSource.secondsSinceLastEventType(.combinedSessionState, eventType: CGEventType(rawValue: ~0)!)` sampler on its existing 60-second timer (`PLAN.md` §7 & §10 T14) and appends `{"ts": <tick_end>, "kind": "activity", "active": <bool>}` to `events.jsonl` when `signals.activity` is consented.
  2. **Safe-Rust fallback in `niwa daemon` (T10):** `tg-signals` provides `PlatformIdleSource` backed on macOS by `/usr/sbin/ioreg -c IOHIDSystem -d 4 -r`, parsing `"HIDIdleTime" = <nanoseconds>` in 100% safe Rust (`unsafe_code = "forbid"`) with zero new crates and zero Accessibility / Input Monitoring TCC prompts.
  3. **Spool deduplication:** Before polling `IdleSource`, `niwa daemon` inspects the already-read `events.jsonl` batch; if a consented `activity` event is already queued for the completed 60-second tick window (for example written by `Niwa.app`), `niwa daemon` skips polling `IdleSource` completely. Even if both write an event in the same tick window, `Garden::catch_up_with_events` combines `active` idempotently (`entry.0 |= ev.active`).

## Decision

Adopt **Option C**:
- Choose the **Swift menu bar app route** for native `CoreGraphics` `CGEventSourceSecondsSinceLastEventType` sampling in T14 (keeping `unsafe_code = "forbid"` and zero Apple FFI crates in Rust).
- Pair it in T10 with a **safe-Rust `/usr/sbin/ioreg` (`IOHIDSystem` `HIDIdleTime`) fallback** behind the `IdleSource` trait in `tg-signals` so standalone CLI + `niwa daemon` users on macOS get full `activity` signal support immediately in M2 without requiring `Niwa.app`.

## Consequences

- `Cargo.toml` keeps `unsafe_code = "forbid"` across the entire Rust workspace and adds zero third-party dependencies.
- Both CLI-only macOS installs (`niwa` formula) and companion menu-bar installs (`niwa-garden` cask) support the `activity` signal seamlessly without TCC permission prompts.
- T14 (`macos/TerminalGarden`) will host the in-process `CGEventSource.secondsSinceLastEventType` sampler on its 60-second timer, writing the same `WorkEvent::Activity` JSON line schema (`{"ts": ..., "kind": "activity", "active": ...}`).
