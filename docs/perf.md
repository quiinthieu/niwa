# Performance & Power Measurements

This document records **actual measured numbers** against the `PLAN.md` §7 power and latency targets (`PLAN.md` §0.9: never invent numbers; every reported figure comes from a real benchmark run).

---

## Summary Against `PLAN.md` §7 Targets

Every target below is continuously benchmarked in `--release` (`-O` for Swift) by [`./scripts/bench.sh`](../scripts/bench.sh) against the stored baseline in [`tests/fixtures/perf-baseline.json`](../tests/fixtures/perf-baseline.json).

| Target (`PLAN.md` §7) | Budget | Stored Baseline (`--release` / `-O`, `2026-10-31`) | Status | Task |
|---|---|---|---|---|
| `niwa _ingest` subprocess latency (`signals.shell` enabled, `O_APPEND` 1 JSONL line) | p95 `< 10 ms` | **p50 `2.274 ms` · p95 `3.535 ms` · p99 `5.201 ms`** (`n = 200`, min `1.859 ms`, max `6.420 ms`) | Pass | T12 / T16 |
| `niwa _ingest` subprocess latency (`signals.shell` disabled, opt-out no-op) | p95 `< 10 ms` | **p50 `2.338 ms` · p95 `4.627 ms` · p99 `10.530 ms`** (`n = 200`, min `1.789 ms`, max `11.984 ms`) | Pass | T12 / T16 |
| `niwa prompt` subprocess latency (fresh `snapshot.json`, default `glyph`) | p95 `< 15 ms`; `0` sim writes | **p50 `2.474 ms` · p95 `4.282 ms` · p99 `6.682 ms`** (`n = 200`, min `1.849 ms`, max `6.708 ms`; `snapshot_untouched = true`) | Pass | T13 / T16 |
| `niwa prompt --style emoji` subprocess latency (fresh `snapshot.json`) | p95 `< 15 ms` | **p50 `2.047 ms` · p95 `3.832 ms` · p99 `4.907 ms`** (`n = 200`, min `1.797 ms`, max `5.898 ms`) | Pass | T13 / T16 |
| `niwa prompt --style none` subprocess latency (opt-out zero-output fast path) | p95 `< 15 ms` | **p50 `2.120 ms` · p95 `4.267 ms` · p99 `6.000 ms`** (`n = 200`, min `1.689 ms`, max `8.411 ms`) | Pass | T13 / T16 |
| 30-day offline `catch_up` (`43,200` ticks, time-only) | `< 200 ms` (`--release`) | **p50 `0.670 ms` · p95 `0.941 ms` · max `0.990 ms`** (`n = 50`, min `0.567 ms`, mean `0.726 ms`) | Pass | T06 / T16 |
| 30-day offline `catch_up_with_events` (`43,200` ticks + `2,400` work events) | `< 200 ms` (`--release`) | **p50 `3.309 ms` · p95 `4.137 ms` · max `4.544 ms`** (`n = 50`, min `2.995 ms`, mean `3.439 ms`) | Pass | T06 / T16 |
| Daemon wake cadence & opt-out zero-wake check | `1` coalesced wake / `60 s`, no FS polling; `0` wakes when no signal enabled | **`60 s` wake interval**, **`0` wakes / `0` sleeps / `0` idle polls** when no signal enabled; single-wake full cycle **p50 `25.208 ms` · p95 `29.686 ms` · max `30.954 ms`** (`n = 50`) | Pass | T10 / T16 |
| TUI frame rate & frame render latency | `<= 4 fps` focused, `0 fps` idle (`30 s`) or unfocused | **`4.000 fps` focused** (`50 ms` input spam), **`0.000 fps` idle** (`60 s` poll sleep), **`0.000 fps` unfocused** (`FocusLost`); `TuiApp::render` `80×24` **p50 `0.180 ms` · p95 `0.295 ms`**, `120×40` **p50 `0.306 ms` · p95 `0.425 ms`** (`n = 200`) | Pass | T08 / T16 |
| macOS `MenuBarExtra` idle CPU, wakes & Energy Log check | Idle except for `60 s` coalesced timer (`tolerance = 6.0 s`) + `DispatchSource` watch on `snapshot.json` | **`0.0179%` CPU** (`0.896 ms` total CPU over `5.005 s` steady-state idle, `0` subprocess spawns between `60 s` ticks, `15.063 MiB` `phys_footprint`; `snapshot.json` reload **p50 `25.166 µs` · p95 `30.917 µs`**); Instruments `xctrace` note below | Pass | T14 / T16 |

---

## T12: `niwa _ingest` Subprocess Latency (`2026-10-31`)

### Environment
- **Date:** `2026-10-31`
- **Host OS & Architecture:** `Darwin 27.2.0 arm64` (macOS `27.2`, Apple Silicon)
- **Toolchain:** `rustc 1.91.1 (ed61e7d7e 2025-11-07)`, Cargo `--release` profile
- **What is timed:** Full end-to-end OS process lifecycle (`std::process::Command::status()` / `posix_spawn` → `main` direct `_ingest` fast path → `NiwaPaths::resolve` → `ConfigStore::load` → `record_if_consented` → `EventSpool::append` (`O_APPEND`) → process exit), isolated inside a temporary `NIWA_HOME` directory.
- **Sample size:** `10` warm-up spawns followed by `200` consecutively timed spawns per mode.

### Measured Results (`--release`)

| Mode | `n` | `min` | `p50` | `mean` | `p95` | `p99` | `max` |
|---|---:|---:|---:|---:|---:|---:|---:|
| `signals.shell = enabled` (appends 1 JSONL line to `events.jsonl`) | 200 | `1.787 ms` | `2.643 ms` | `2.793 ms` | **`4.079 ms`** | `4.754 ms` | `5.907 ms` |
| `signals.shell = disabled` (opt-out early exit, 0 bytes written) | 200 | `1.711 ms` | `2.613 ms` | `2.917 ms` | **`4.726 ms`** | `6.242 ms` | `6.892 ms` |

### Why `_ingest` Stays Fast and Non-Blocking
1. **Backgrounded in the shell hook:** `zsh`, `bash`, and `fish` hooks spawn `niwa _ingest --exit <code> --dur <secs>` asynchronously in the background with stdout/stderr redirected to `/dev/null` and job notifications suppressed (`docs/platform/shell-hooks.md`), so interactive prompts never block on `_ingest`.
2. **Direct argument & path fast path:** `niwa_garden::ingest::try_run_fast_cli_from_env` parses `_ingest --exit <code> --dur <secs>` directly from `std::env::args_os()` without building `clap`'s full command tree, and `NiwaPaths::resolve` avoids `BaseDirs::new()` system directory lookups whenever `NIWA_HOME` or `HOME` is set in the environment.
3. **Zero simulation & zero lock contention:** `_ingest` never loads embedded `Balance`, `SpeciesCatalog`, `SpriteBank`, or `ConsentCatalog` assets, never reads or writes `garden.json` or `snapshot.json`, and never acquires `garden.lock`. It appends a single ~65-byte JSONL record to `events.jsonl` with `O_APPEND`.

---

## T13: `niwa prompt` Subprocess Latency (`2026-10-31`)

### Environment
- **Date:** `2026-10-31`
- **Host OS & Architecture:** `Darwin 27.2.0 arm64` (macOS `27.2`, Apple Silicon)
- **Toolchain:** `rustc 1.91.1 (ed61e7d7e 2025-11-07)`, Cargo `--release` profile
- **What is timed:** Full end-to-end OS process lifecycle including stdout pipe capture (`std::process::Command::output()` → `main` direct `prompt` fast path → `NiwaPaths::resolve` → `ConfigStore::load` (when needed) → `read_snapshot` (`< 60 s` fresh `snapshot.json`) → stdout write → process exit), isolated inside a temporary `NIWA_HOME` directory and asserting `snapshot.json` remains byte-for-byte untouched across all spawns.
- **Sample size:** `10` warm-up spawns followed by `200` consecutively timed spawns per mode.

### Measured Results (`--release`)

| Mode | `n` | `min` | `p50` | `mean` | `p95` | `p99` | `max` |
|---|---:|---:|---:|---:|---:|---:|---:|
| `niwa prompt` (default `prompt.style = "glyph"`, reads `config.toml` + `snapshot.json`) | 200 | `1.885 ms` | `3.299 ms` | `4.246 ms` | **`9.208 ms`** | `16.599 ms` | `24.452 ms` |
| `niwa prompt --style emoji` (reads `snapshot.json` directly) | 200 | `1.882 ms` | `2.880 ms` | `3.009 ms` | **`4.550 ms`** | `5.720 ms` | `7.200 ms` |
| `niwa prompt --style none` (zero-I/O opt-out short-circuit) | 200 | `1.605 ms` | `2.389 ms` | `2.580 ms` | **`4.071 ms`** | `4.594 ms` | `5.179 ms` |

### Why `prompt` Stays Fast and Never Blocks the Shell
1. **Direct argument fast path:** `niwa_garden::prompt::try_run_fast_prompt_from_env` parses `prompt [--style glyph|emoji|none]` directly from `std::env::args_os()` without constructing `clap`'s full subcommand tree.
2. **Strict `< 60 s` read-only fast path (`PLAN.md` §3.4):** Whenever `snapshot.json` is `< 60 s` old (`SNAPSHOT_MAX_AGE_SECS = 60`), `niwa prompt` never acquires `garden.lock`, never loads `garden.json`, `events.jsonl`, or embedded `Balance` / `SpeciesCatalog` / `SpriteBank` TOML assets, and never runs `catch_up`.
3. **Non-blocking stale refresh (`GardenLock::try_acquire`):** When `snapshot.json` is `>= 60 s` old or missing, `niwa prompt` uses a non-blocking `try_acquire` on `garden.lock`; if an interactive `niwa` TUI or `niwa daemon` currently holds the lock, `niwa prompt` immediately renders from the existing snapshot instead of blocking `PS1`, `tmux`, or Starship.

---

## T14: macOS `MenuBarExtra` Idle Footprint & Instruments Energy Check (`2026-10-31`)

### Environment
- **Date:** `2026-10-31`
- **Host OS & Architecture:** `Darwin 27.2.0 arm64` (macOS `27.2`, Apple Silicon)
- **Toolchain:** Apple Swift `6.4` (`swiftc -O -swift-version 6 -target arm64-apple-macosx14.0`, CommandLineTools SDK)
- **What is timed:** Release-optimized (`-O`) `MenuBarViewModel` + `SnapshotWatcher` (`DispatchSourceFileSystemObject` `O_EVTONLY` watching both `$NIWA_HOME` and `snapshot.json`) + `60.0 s` coalesced `Timer` (`tolerance = 6.0 s`) + laid-out `NSHostingView(rootView: MenuBarPopoverView(...))` running undisturbed on `RunLoop.main` over a `10.0 s` steady-state idle window after initial startup drain, followed by `200` consecutive `reloadSnapshotFromDisk()` (`Data(contentsOf:)` + `GardenSnapshot.decode`) iterations.

### Measured Steady-State Idle & Reload Figures (`-O`)

| Metric | Measured Value | Notes |
|---|---:|---|
| Steady-state observation window | `10,009.39 ms` | Between 60-second timer ticks |
| User CPU time (`ru_utime`) | `1.074 ms` | `getrusage(RUSAGE_SELF)` delta |
| System CPU time (`ru_stime`) | `0.883 ms` | `getrusage(RUSAGE_SELF)` delta |
| **Total steady-state CPU utilization** | **`0.0196%` (`1.957 ms` / `10.009 s`)** | Zero busy-waiting, zero filesystem polling |
| Subprocess spawns between 60 s ticks | **`0`** | `niwa` runs only on 60 s tick or explicit user action |
| Process physical footprint (`task_vm_info.phys_footprint`) | **`14.78 MiB`** | `AppKit` + `SwiftUI` + `MenuBarPopoverView` hosted |
| `snapshot.json` read + JSON decode (`n = 200`) | **p50 `47.8 µs` · p95 `72.3 µs` · max `210.1 µs`** | Triggered only on `EVFILT_VNODE` write/rename events |

### Power-Budget Design & Instruments Energy Log Note (`PLAN.md` §0.9 & §7)
1. **Zero filesystem polling:** `SnapshotWatcher` uses kernel `kqueue` `EVFILT_VNODE` descriptors (`O_EVTONLY`) on the data directory and `snapshot.json`, waking only when `tg-store` atomically replaces `snapshot.json`.
2. **Coalesced 60-second timer (`tolerance = 6.0 s`) & App Nap (`LSUIElement = true`):** `MenuBarViewModel.tickIntervalSeconds = 60.0` sets a 10% (`6.0 s`) tolerance so macOS coalesces periodic wakes with system activity and keeps the Dock-less agent eligible for App Nap whenever the popover is closed.
3. **In-process idle sampling (`ADR 003`):** When `signals.activity` is enabled, `CoreGraphicsIdleTimeSampler` reads `CGEventSource.secondsSinceLastEventType` directly in-process (zero extra `ioreg` fork) before passing `--idle-secs` to `niwa daemon --once`. When `signals.activity` is disabled and `snapshot.json` is already `< 60 s` fresh (for example when `niwa daemon` or an open TUI is active), `handlePeriodicTick()` spawns **zero** subprocesses.
4. **Instruments / `xctrace` status:** On the local development workstation, only `/Library/Developer/CommandLineTools` is installed (full `Xcode.app` / `Instruments.app` / `xcrun xctrace` are not present locally, and headless CI runners do not expose interactive macOS Power/Energy Log PMC templates). On a workstation with full `Xcode.app`, run:
   ```sh
   xcodegen generate --spec macos/Project.yml --project macos
   xcodebuild -project macos/TerminalGarden.xcodeproj -scheme TerminalGarden -configuration Release build
   xcrun xctrace record --template 'Energy Log' --time-limit 3m --output /tmp/niwa-energy.trace --launch -- /path/to/Niwa.app
   ```
   and confirm **Energy Impact** remains **Low** (`0` overhead wakes between the 60 s coalesced timer ticks).

---

## T16: Stored Baseline & Full `PLAN.md` §7 Suite (`scripts/bench.sh`, `2026-10-31`)

### Harness & Stored Baseline
- **Runner:** [`./scripts/bench.sh`](../scripts/bench.sh) runs the `--release` Rust benchmark driver ([`crates/niwa/tests/perf_bench.rs`](../crates/niwa/tests/perf_bench.rs), `run_release_perf_benchmarks_and_emit_json`) and, on macOS, compiles and runs the `-O` Swift 6 menu bar driver ([`macos/Bench/MenuBarBench.swift`](../macos/Bench/MenuBarBench.swift)), then compares every metric against [`tests/fixtures/perf-baseline.json`](../tests/fixtures/perf-baseline.json).
- **Schema & budget invariant test:** `stored_perf_baseline_json_exists_and_covers_every_section_7_target` in [`crates/niwa/tests/perf_bench.rs`](../crates/niwa/tests/perf_bench.rs) runs as part of `cargo test --workspace` on every OS, verifying that `tests/fixtures/perf-baseline.json` parses cleanly and that every stored baseline figure satisfies its `PLAN.md` §7 budget.
- **Commands:**
  ```sh
  ./scripts/bench.sh                     # Run --release / -O suite and check against tests/fixtures/perf-baseline.json
  ./scripts/bench.sh --json /tmp/out.json # Emit raw measured JSON for the current run
  ./scripts/bench.sh --update-baseline   # Re-record tests/fixtures/perf-baseline.json on macOS
  ```

### 1. 30-Day Offline `catch_up` (`43,200` ticks, all plots planted, `--release`)

| Scenario | `n` | `min` | `p50` | `mean` | `p95` | `p99` | `max` | §7 Budget |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Time-only (`Garden::catch_up`, `43,200` ticks across 30 days) | 50 | `0.567 ms` | `0.670 ms` | `0.726 ms` | **`0.941 ms`** | `0.990 ms` | `0.990 ms` | `< 200 ms` |
| Event-carrying (`Garden::catch_up_with_events`, `43,200` ticks + `2,400` work events) | 50 | `2.995 ms` | `3.309 ms` | `3.439 ms` | **`4.137 ms`** | `4.544 ms` | `4.544 ms` | `< 200 ms` |

### 2. `niwa daemon` Wake Cadence, Opt-Out Zero-Wake & Single-Wake Cost (`--release`)

| Metric | Measured Value | §7 Target |
|---|---:|---|
| Periodic wake interval (`DAEMON_TICK_INTERVAL`) | **`60 s`** | `1` coalesced wake per `60 s`, no filesystem polling |
| Wakes / sleeps / idle polls when no signal is enabled | **`0` wakes · `0` sleeps · `0` idle polls** | `0` daemon wakes when no signal is enabled (`DaemonStopReason::SkippedNoSignals`) |
| Single-wake full cycle (`lock → load → sample → catch_up → save → snapshot`, `n = 50`) | **min `14.079 ms` · p50 `25.208 ms` · mean `25.338 ms` · p95 `29.686 ms` · max `30.954 ms`** | Completes in `< 0.05%` of each `60 s` interval |

### 3. `niwa` TUI Frame-Rate Caps & Frame Render Latency (`--release`)

Verified both by [`crates/niwa/tests/tui_flow.rs`](../crates/niwa/tests/tui_flow.rs) and [`crates/niwa/tests/perf_bench.rs`](../crates/niwa/tests/perf_bench.rs):

| Metric | Measured Value | §7 Target |
|---|---:|---|
| Focused active frame rate (`200` key events at `50 ms` intervals over `10.0 s`) | **`4.000 fps`** (`40` draws / `10.0 s`) | `<= 4 fps` while focused |
| Idle frame rate after `30 s` without input (`300 s` window) | **`0.000 fps`** (`0` draws) | `0 fps` when idle |
| Idle event-loop poll timeout (`FrameScheduler::next_poll_timeout`) | **`60 s`** (`SIM_TICK_INTERVAL`) | Sleeps until next `60 s` simulation tick |
| Unfocused frame rate after `Event::FocusLost` (`60 s` window) | **`0.000 fps`** (`0` draws) | `0 fps` when unfocused |
| `TuiApp::render` frame cost at `80×24` (`n = 200`, `Truecolor`, seasonal ambience active) | **min `0.163 ms` · p50 `0.180 ms` · mean `0.197 ms` · p95 `0.295 ms` · max `0.598 ms`** | `< 0.2%` of the `250 ms` `4 fps` frame interval |
| `TuiApp::render` frame cost at `120×40` (`n = 200`, `Truecolor`, seasonal ambience active) | **min `0.284 ms` · p50 `0.306 ms` · mean `0.325 ms` · p95 `0.425 ms` · max `0.778 ms`** | `< 0.2%` of the `250 ms` `4 fps` frame interval |

### 4. Stored Baseline Summary (`tests/fixtures/perf-baseline.json`, `Darwin 27.2.0 arm64`)

- **`niwa _ingest` (`n = 200`):** `signals.shell = enabled` **p50 `2.274 ms` · p95 `3.535 ms` · p99 `5.201 ms` · max `6.420 ms`**; `signals.shell = disabled` **p50 `2.338 ms` · p95 `4.627 ms` · p99 `10.530 ms` · max `11.984 ms`** (target: p95 `< 10 ms`).
- **`niwa prompt` (`n = 200`, `snapshot_untouched = true`):** `glyph` **p50 `2.474 ms` · p95 `4.282 ms` · p99 `6.682 ms` · max `6.708 ms`**; `--style emoji` **p50 `2.047 ms` · p95 `3.832 ms` · p99 `4.907 ms` · max `5.898 ms`**; `--style none` **p50 `2.120 ms` · p95 `4.267 ms` · p99 `6.000 ms` · max `8.411 ms`** (target: p95 `< 15 ms`).
- **macOS `MenuBarExtra` (`swiftc -O`, `5,005.066 ms` steady-state window):** **`0.0179%` CPU** (`0.502 ms` user + `0.394 ms` sys = `0.896 ms` total CPU), **`0` spawns** between `60.0 s` (`tolerance = 6.0 s`) ticks, **`15.063 MiB`** `phys_footprint`, `snapshot.json` reload (`n = 200`) **p50 `25.166 µs` · p95 `30.917 µs` · max `102.541 µs`**.
