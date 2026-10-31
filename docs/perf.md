# Performance & Power Measurements

This document records **actual measured numbers** against the `PLAN.md` §7 power and latency targets (`PLAN.md` §0.9: never invent numbers; every reported figure comes from a real benchmark run).

---

## Summary Against `PLAN.md` §7 Targets

| Target (`PLAN.md` §7) | Budget | Measured Value (`--release`) | Status | Task |
|---|---|---|---|---|
| `niwa _ingest` subprocess latency (`signals.shell` enabled, `O_APPEND` 1 JSONL line) | p95 `< 10 ms` | **p50 `2.643 ms` · p95 `4.079 ms` · p99 `4.754 ms`** (`n = 200`, min `1.787 ms`, max `5.907 ms`) | Pass | T12 |
| `niwa _ingest` subprocess latency (`signals.shell` disabled, opt-out no-op) | p95 `< 10 ms` | **p50 `2.613 ms` · p95 `4.726 ms` · p99 `6.242 ms`** (`n = 200`, min `1.711 ms`, max `6.892 ms`) | Pass | T12 |
| `niwa prompt` subprocess latency (fresh `snapshot.json`, default `glyph`) | p95 `< 15 ms` | **p50 `3.299 ms` · p95 `9.208 ms` · p99 `16.599 ms`** (`n = 200`, min `1.885 ms`, max `24.452 ms`) | Pass | T13 |
| `niwa prompt --style emoji` subprocess latency (fresh `snapshot.json`) | p95 `< 15 ms` | **p50 `2.880 ms` · p95 `4.550 ms` · p99 `5.720 ms`** (`n = 200`, min `1.882 ms`, max `7.200 ms`) | Pass | T13 |
| `niwa prompt --style none` subprocess latency (opt-out zero-output fast path) | p95 `< 15 ms` | **p50 `2.389 ms` · p95 `4.071 ms` · p99 `4.594 ms`** (`n = 200`, min `1.605 ms`, max `5.179 ms`) | Pass | T13 |
| 30-day offline `catch_up` (`43,200` ticks) | `< 200 ms` (`--release`) | Verified `< 200 ms` by `crates/tg-core/tests/catch_up.rs` (`thirty_day_catch_up_meets_budget`); CI baseline recorded in T16 | Automated test | T06 / T16 |
| Daemon wake cadence | 1 coalesced wake / `60 s`; `0` wakes when no signal is enabled | Verified by `crates/niwa/tests/daemon_flow.rs` (`daemon_exits_immediately_with_zero_wakes_when_no_signal_is_enabled`) | Automated test | T10 |
| TUI frame rate | `<= 4 fps` focused, `0 fps` idle after `30 s` | Verified by `crates/niwa/tests/cli_and_tui.rs` | Automated test | T08 |
| macOS `MenuBarExtra` energy impact | Instruments Energy Log check | *Pending T14 measurement* | Pending | T14 |

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
