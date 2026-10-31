# Platform Verification: macOS Menu Bar Companion (`MenuBarExtra`, `SMAppService` & `DispatchSource`)

- **Verified on:** `2026-10-31`
- **Plan references:** `PLAN.md` §0.8, §3, §3.1, §4.3, §6, §7, §8, §9 & §10 T14
- **Related ADR & platform docs:**
  - [`docs/decisions/003-macos-idle-time-source.md`](../decisions/003-macos-idle-time-source.md)
  - [`docs/platform/idle-time.md`](./idle-time.md)
  - [`docs/snapshot-schema.md`](../snapshot-schema.md)

---

## 1. `SwiftUI.MenuBarExtra` & Minimum macOS Deployment Target (`macOS 14.0+`)

- **SwiftUI Scene API:** `MenuBarExtra` (`init(_:content:)` / `init(isInserted:content:label:)`) paired with `.menuBarExtraStyle(.window)` (`WindowMenuBarExtraStyle`).
- **Platform availability (verified `2026-10-31` against Apple Developer Documentation & macOS Swift 6 SDK):**
  - `MenuBarExtra` and `.menuBarExtraStyle(.window)` were introduced in **macOS 13.0 (Ventura)**.
  - Swift's `Observation` framework (`@Observable` macro) was introduced in **macOS 14.0 (Sonoma)**.
  - Targeting **`MACOSX_DEPLOYMENT_TARGET = 14.0`** with **`SWIFT_VERSION = 6.0`** satisfies both `MenuBarExtra(.window)` and `@Observable` with strict Swift 6 concurrency checking (`@MainActor` isolation) and zero third-party Swift packages (`PLAN.md` §3.3).
- **Menu-bar-only agent bundle (`LSUIElement`):**
  - Setting `LSUIElement = true` (`Application is agent (UIElement)`) in `Info.plist` suppresses the Dock icon and Cmd+Tab switcher entry while keeping the `MenuBarExtra` glyph and `.window` popover active.
- **Title behaviour (`menubar.show_title`):**
  - When `menubar.show_title = true` (default in `config.toml`), the status item displays the 1-cell compact glyph from `snapshot.json` (e.g. `◈`, `◎`, `◌`, or `*` in ASCII mode).
  - When `menubar.show_title = false`, the status item omits the text title and renders a minimal leaf SF Symbol (`leaf` / `leaf.fill`) so the menu bar item remains clickable.
- **Official documentation URLs (verified `2026-10-31`):**
  - `https://developer.apple.com/documentation/swiftui/menubarextra`
  - `https://developer.apple.com/documentation/swiftui/scene/menubarextrastyle(_:)`
  - `https://developer.apple.com/documentation/swiftui/menubarextrastyle/window`
  - `https://developer.apple.com/documentation/bundleresources/information-property-list/lsuielement`

---

## 2. Launch at Login via `ServiceManagement.SMAppService`

- **Framework & Type:** `import ServiceManagement` → `SMAppService.mainApp`.
- **Platform availability (verified `2026-10-31`):** **macOS 13.0+**, replacing deprecated `SMLoginItemSetEnabled` without requiring an embedded helper app bundle.
- **Verified API surface:**
  - `SMAppService.mainApp.status -> SMAppService.Status`:
    - `.notRegistered` — Launch at Login is off.
    - `.enabled` — Registered and active; macOS launches `Niwa.app` automatically on user login.
    - `.requiresApproval` — Registered, awaiting user approval in **System Settings → General → Login Items** (`SMAppService.openSystemSettingsLoginItems()` opens the pane directly).
    - `.notFound` — Bundle registration unavailable (for example when running an unbundled binary outside `/Applications` during local CLI/test runs); surfaced cleanly in UI without crashing.
  - `try SMAppService.mainApp.register()` — Opts into Launch at Login (never called automatically on first launch; strictly user-initiated).
  - `try SMAppService.mainApp.unregister()` — Removes Launch at Login registration.
- **Testability:** Isolated behind a `LoginItemManaging` protocol in `macos/Sources/TerminalGarden/LaunchAtLoginController.swift` so `XCTest` suites exercise `.notRegistered`, `.enabled`, `.requiresApproval`, `.notFound`, and registration errors deterministically without mutating the developer's or CI runner's real macOS Login Items.
- **Official documentation URLs (verified `2026-10-31`):**
  - `https://developer.apple.com/documentation/servicemanagement/smappservice`
  - `https://developer.apple.com/documentation/servicemanagement/smappservice/mainapp`
  - `https://developer.apple.com/documentation/servicemanagement/smappservice/status-swift.enum`
  - `https://developer.apple.com/documentation/servicemanagement/smappservice/register()`
  - `https://developer.apple.com/documentation/servicemanagement/smappservice/unregister()`
  - `https://developer.apple.com/documentation/servicemanagement/smappservice/opensystemsettingsloginitems()`

---

## 3. Zero-Polling `snapshot.json` Watch via `DispatchSourceFileSystemObject`

- **Mechanism:** `DispatchSource.makeFileSystemObjectSource(fileDescriptor:eventMask:queue:)` (`kqueue` `EVFILT_VNODE`) using `open(path, O_EVTONLY)`.
- **Atomic-rename caveat & verified handling:**
  - `tg_store::snapshot::write_snapshot` writes a sibling temporary file, `fsync`s it, and atomically `rename(2)`s it over `snapshot.json`.
  - Under `EVFILT_VNODE`, an `O_EVTONLY` descriptor opened on the old `snapshot.json` inode receives `.delete` / `.rename` when replaced and no longer tracks the new inode; moreover, `snapshot.json` may not exist yet before the user's first garden command.
  - `SnapshotWatcher` therefore watches the **parent data directory** (`~/Library/Application Support/Niwa` or `$NIWA_HOME`) with `O_EVTONLY` for `[.write, .rename, .delete]` **and** attaches/re-arms an `O_EVTONLY` source on `snapshot.json` itself (`[.write, .extend, .attrib, .delete, .rename]`), coalescing reload callbacks onto `@MainActor`.
  - Between filesystem writes and the 60-second coalesced timer (`Timer.scheduledTimer` with `tolerance = 6.0` s for macOS timer coalescing / App Nap), the app performs **zero polling**.
- **Official documentation URL (verified `2026-10-31`):**
  - `https://developer.apple.com/documentation/dispatch/dispatchsource/makefilesystemobjectsource(filedescriptor:eventmask:queue:)`

---

## 4. Thin-Client CLI Contract & Native Idle-Time Sampler (`ADR 003`)

- **No game logic and no direct disk writes in Swift (`PLAN.md` §0.6 & §3):**
  - `Niwa.app` reads `snapshot.json` (tolerating unknown JSON fields) and shells out to `niwa` (`Foundation.Process`, zero `URLSession` / `Network` imports):
    - **Harvest All:** `niwa harvest --all`
    - **Pause / Resume Signals:** `niwa config set signals.paused true` / `niwa config set signals.paused false` (queried via `niwa config get ...`, honouring `PLAN.md` §6)
    - **Open TUI in Terminal:** `NSWorkspace` / `/usr/bin/open -a Terminal <niwa-binary>` (launches `niwa` directly as a UNIX executable in `Terminal.app` without writing temporary `.command` scripts or triggering AppleEvents TCC prompts)
    - **60 s Idle-Time Sampler (`ADR 003`):** On its 60-second coalesced timer, when `signals.activity` is enabled and not paused, `Niwa.app` samples `CGEventSource.secondsSinceLastEventType(.combinedSessionState, eventType: CGEventType(rawValue: ~0)!)` in-process and passes `--idle-secs <secs>` to `niwa daemon --once --idle-secs <secs>`, delegating consent verification, spool persistence, catch-up, and `snapshot.json` atomic replacement exclusively to Rust (`tg-signals` + `tg-store`).
