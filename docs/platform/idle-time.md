# Platform Verification: System User-Input Idle Time (`activity` Signal)

- **Verified on:** `2026-10-31`
- **Plan references:** `PLAN.md` §0.8, §5 (`activity` row), §7, §10 T10 & T14
- **Related ADR:** [`docs/decisions/003-macos-idle-time-source.md`](../decisions/003-macos-idle-time-source.md)

## Overview

When the user explicitly opts in via `niwa signals enable activity`, Niwa checks how long ago the last local keyboard/pointer input occurred once per 60-second tick (`TICK_SECS = 60`). If the idle duration is strictly less than `60 s`, the completed tick is recorded as `active: true`; otherwise `active: false`. When an unsupported or headless session cannot report idle duration, `IdleSource::sample_idle_duration` returns `None` and no `WorkEvent::Activity` is fabricated.

---

## macOS

### 1. Native `CoreGraphics` API (Swift Menu Bar Companion — T14)

- **C Function:** `CGEventSourceSecondsSinceLastEventType(CGEventSourceStateID stateID, CGEventType eventType) -> CFTimeInterval`
- **Swift 6 Method:** `CGEventSource.secondsSinceLastEventType(_ stateID: CGEventSourceStateID, eventType: CGEventType) -> CFTimeInterval`
- **Parameters:**
  - `stateID`: `.combinedSessionState` (`kCGEventSourceStateCombinedSessionState = 0`) — reflects combined HID + remote/session input across the logged-in GUI session.
  - `eventType`: `CGEventType(rawValue: ~0)!` (`kCGAnyInputEventType = ~0`) — matches the most recent keyboard, mouse, or trackpad input event of any type.
- **Return unit:** Seconds as `CFTimeInterval` (`Double`).
- **Permissions / TCC:** Requires **no** Accessibility (`AXIsProcessTrusted`) or Input Monitoring permission because it reads only an aggregate elapsed-time scalar and never inspects event payloads.
- **Official documentation URLs (verified `2026-10-31`):**
  - `https://developer.apple.com/documentation/coregraphics/1408790-cgeventsourcesecondssincelasteve`
  - `https://developer.apple.com/documentation/coregraphics/cgeventsource/1408790-secondssincelasteventtype`
  - `https://developer.apple.com/documentation/coregraphics/cgeventsourcestateid/combinedsessionstate`

### 2. Safe-Rust `IOHIDSystem` Fallback (`niwa daemon` — T10)

- **Command:** `/usr/sbin/ioreg -c IOHIDSystem -d 4 -r`
- **Registry property:** `"HIDIdleTime" = <u64>`
- **Return unit:** **Nanoseconds** (`u64`), converted via `Duration::from_nanos(ns)`.
- **Permissions / TCC:** Unprivileged IOKit registry read; requires **no** Accessibility or Input Monitoring permission and `0` `unsafe` Rust blocks (verified directly on macOS on `2026-10-31`).
- **Official documentation references (verified `2026-10-31`):**
  - macOS `ioreg(8)` manual page (`/usr/sbin/ioreg`)
  - Apple IOKit Fundamentals (`IOHIDSystem` registry plane): `https://developer.apple.com/library/archive/documentation/DeviceDrivers/Conceptual/IOKitFundamentals/`

---

## Linux (Best-Effort Session Probes — T10)

Because Linux desktop sessions span Wayland compositors, X11 servers, and headless terminals/WSL—and `PLAN.md` §3.3 keeps the Rust dependency tree free of D-Bus or X11 FFI crates—`PlatformIdleSource` probes available user-session tools in order and gracefully returns `None` when none apply:

### 1. GNOME Mutter IdleMonitor (Wayland & X11)

- **D-Bus call:**
  ```sh
  gdbus call --session \
    --dest org.gnome.Mutter.IdleMonitor \
    --object-path /org/gnome/Mutter/IdleMonitor/Core \
    --method org.gnome.Mutter.IdleMonitor.GetIdletime
  ```
- **Output format:** `(uint64 <milliseconds>,)`
- **Return unit:** **Milliseconds** (`u64`), converted via `Duration::from_millis(ms)`.
- **Official interface specification URL (verified `2026-10-31`):**
  - `https://gitlab.gnome.org/GNOME/mutter/-/blob/main/data/dbus-interfaces/org.gnome.Mutter.IdleMonitor.xml`

### 2. X11 `MIT-SCREEN-SAVER` Extension via `xprintidle` (when `$DISPLAY` is set)

- **Command:** `xprintidle` (queries `XScreenSaverQueryInfo` on the active X11 `$DISPLAY`)
- **Output format:** Plain unsigned integer `<milliseconds>\n` on `stdout`.
- **Return unit:** **Milliseconds** (`u64`), converted via `Duration::from_millis(ms)`.
- **Ordering rationale:** Probed ahead of `org.freedesktop.ScreenSaver` whenever `$DISPLAY` is non-empty because `MIT-SCREEN-SAVER` idle time is unambiguously reported in milliseconds across all X11 desktops and window managers.
- **Official specification & upstream URLs (verified `2026-10-31`):**
  - X11 Screen Saver Extension Protocol (`MIT-SCREEN-SAVER`): `https://www.x.org/releases/current/doc/scrnsaverproto/saver.html`
  - `xprintidle` upstream: `https://github.com/g0hl1n/xprintidle`

### 3. `org.freedesktop.ScreenSaver.GetSessionIdleTime` (freedesktop / KDE Wayland Fallback)

- **D-Bus call:**
  ```sh
  gdbus call --session \
    --dest org.freedesktop.ScreenSaver \
    --object-path /ScreenSaver \
    --method org.freedesktop.ScreenSaver.GetSessionIdleTime
  ```
- **Output format:** `(uint32 <value>,)`
- **Unit & KDE Plasma caveat (verified `2026-10-31`):**
  - The formal `org.freedesktop.ScreenSaver` specification defines `GetSessionIdleTime` in **seconds** (`Duration::from_secs(value)`).
  - KDE Plasma's `kscreenlocker` / `KIdleTime` implementation of `org.freedesktop.ScreenSaver.GetSessionIdleTime` historically returns **milliseconds** (`Duration::from_millis(value)`).
  - `tg-signals` checks `XDG_CURRENT_DESKTOP`: when it contains `KDE` (case-insensitive), `value` is interpreted as **milliseconds**; otherwise it is interpreted as **seconds** per the freedesktop specification.
- **Official specification references (verified `2026-10-31`):**
  - freedesktop Idle Inhibition / ScreenSaver D-Bus interface (`org.freedesktop.ScreenSaver`): `https://specifications.freedesktop.org/idle-inhibit-spec/latest/`
  - KDE `kscreenlocker` D-Bus `org.freedesktop.ScreenSaver.xml`: `https://invent.kde.org/plasma/kscreenlocker`
