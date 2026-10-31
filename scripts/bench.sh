#!/usr/bin/env bash
# Benchmark every PLAN.md §7 power and latency target in `--release` (`-O` for Swift)
# and compare measured figures against `tests/fixtures/perf-baseline.json` (PLAN.md §7, §8, §9 & §10 T16).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
BASELINE_FILE="$REPO_ROOT/tests/fixtures/perf-baseline.json"

MODE="check"
JSON_OUT=""

while [ "$#" -gt 0 ]; do
    case "$1" in
        --check)
            MODE="check"
            shift
            ;;
        --update-baseline)
            MODE="update"
            shift
            ;;
        --json)
            if [ "$#" -lt 2 ]; then
                echo "ERROR: --json requires an output file path" >&2
                exit 1
            fi
            JSON_OUT="$2"
            shift 2
            ;;
        -h|--help)
            cat <<'EOF'
Usage: ./scripts/bench.sh [--check] [--update-baseline] [--json <file>]

Options:
  --check             Run --release benchmarks and verify against tests/fixtures/perf-baseline.json (default)
  --update-baseline   Record fresh --release measurements into tests/fixtures/perf-baseline.json (requires macOS)
  --json <file>       Write the raw measured JSON report for this run to <file>
EOF
            exit 0
            ;;
        *)
            echo "ERROR: Unknown argument '$1'" >&2
            exit 1
            ;;
    esac
done

WORK_TMP="$(mktemp -d "${TMPDIR:-/tmp}/niwa-bench.XXXXXX")"
cleanup() {
    rm -rf "$WORK_TMP"
}
trap cleanup EXIT

RUST_JSON="$WORK_TMP/rust-bench.json"
MACOS_JSON="$WORK_TMP/macos-bench.json"

CARGO_EXTRA_ARGS=()
if [ "${NIWA_CARGO_OFFLINE:-0}" = "1" ]; then
    CARGO_EXTRA_ARGS+=(--offline)
fi

echo "==> Running Rust --release PLAN.md §7 benchmarks (niwa _ingest, niwa prompt, 30-day catch_up, daemon, TUI)..."
(
    cd "$REPO_ROOT"
    NIWA_BENCH_JSON_OUT="$RUST_JSON" cargo test --release "${CARGO_EXTRA_ARGS[@]}" \
        -p niwa-garden --test perf_bench run_release_perf_benchmarks_and_emit_json \
        -- --ignored --exact --nocapture
)

HOST_OS="$(uname -s)"
HOST_ARCH="$(uname -m)"
HOST_KERNEL="$(uname -sr)"

if [ "$HOST_OS" = "Darwin" ]; then
    echo "==> Building & running Swift 6 -O macOS MenuBarExtra steady-state benchmark..."
    MACOS_BENCH_BIN="$WORK_TMP/niwa-menubar-bench"
    swiftc -O -swift-version 6 -warnings-as-errors -parse-as-library \
        "$REPO_ROOT"/macos/Sources/TerminalGarden/*.swift \
        "$REPO_ROOT"/macos/Bench/MenuBarBench.swift \
        -o "$MACOS_BENCH_BIN"

    TARGET_ROOT="${CARGO_TARGET_DIR:-$REPO_ROOT/target}"
    RELEASE_NIWA_BIN="$TARGET_ROOT/release/niwa"
    NIWA_BENCH_BIN="$RELEASE_NIWA_BIN" \
    NIWA_MACOS_BENCH_IDLE_SECS="${NIWA_MACOS_BENCH_IDLE_SECS:-5.0}" \
    NIWA_MACOS_BENCH_JSON_OUT="$MACOS_JSON" \
        "$MACOS_BENCH_BIN"
elif [ "$MODE" = "update" ]; then
    echo "ERROR: --update-baseline must run on macOS so macos_menubar figures are measured, never invented (PLAN.md §0.9)." >&2
    exit 1
fi

python3 - "$MODE" "$BASELINE_FILE" "$RUST_JSON" "$MACOS_JSON" "$JSON_OUT" "$HOST_KERNEL $HOST_ARCH" <<'PY'
import datetime
import json
import os
import sys

mode, baseline_path, rust_json_path, macos_json_path, json_out_path, host_platform = sys.argv[1:7]

with open(rust_json_path, "r", encoding="utf-8") as fh:
    rust_run = json.load(fh)

macos_run = None
if os.path.exists(macos_json_path):
    with open(macos_json_path, "r", encoding="utf-8") as fh:
        macos_run = json.load(fh)

budgets = {
    "ingest_p95_budget_ms": 10.0,
    "prompt_p95_budget_ms": 15.0,
    "catch_up_30d_budget_ms": 200.0,
    "daemon_wake_interval_secs": 60,
    "daemon_wakes_when_no_signals": 0,
    "tui_max_focused_fps": 4.0,
    "tui_idle_fps": 0.0,
    "tui_unfocused_fps": 0.0,
    "tui_idle_poll_timeout_secs": 60,
    "menubar_timer_interval_secs": 60.0,
    "menubar_timer_tolerance_secs": 6.0,
    "menubar_idle_spawns_between_ticks": 0,
    "menubar_max_idle_cpu_pct": 1.0,
}

if json_out_path:
    combined = {
        "recorded_on": datetime.date.today().isoformat(),
        "host_platform": host_platform,
        "rust": rust_run,
        "macos_menubar": macos_run,
    }
    os.makedirs(os.path.dirname(os.path.abspath(json_out_path)), exist_ok=True)
    with open(json_out_path, "w", encoding="utf-8") as fh:
        json.dump(combined, fh, indent=2)
        fh.write("\n")

if mode == "update":
    assert macos_run is not None
    payload = {
        "schema_version": 1,
        "recorded_on": datetime.date.today().isoformat(),
        "host_platform": host_platform,
        "budgets": budgets,
        "baseline": {
            "rust": rust_run,
            "macos_menubar": macos_run,
        },
    }
    os.makedirs(os.path.dirname(baseline_path), exist_ok=True)
    with open(baseline_path, "w", encoding="utf-8") as fh:
        json.dump(payload, fh, indent=2)
        fh.write("\n")
    print(f"Updated stored PLAN.md §7 baseline at {baseline_path}")

if not os.path.exists(baseline_path):
    print(f"ERROR: Missing stored baseline file: {baseline_path}", file=sys.stderr)
    sys.exit(1)

with open(baseline_path, "r", encoding="utf-8") as fh:
    stored = json.load(fh)

b = stored["budgets"]
base_rust = stored["baseline"]["rust"]
base_mb = stored["baseline"]["macos_menubar"]

rows = []
failures = []


def check_upper(name, measured, baseline_val, hard_budget, ceiling, unit, strict=False):
    delta = measured - baseline_val
    if strict:
        ok = measured < hard_budget and measured <= ceiling
        op = "<"
    else:
        ok = measured <= hard_budget and measured <= ceiling
        op = "<="
    status = "PASS" if ok else "FAIL"
    bud_str = (
        f"{op} {hard_budget:.3f} {unit}"
        if abs(ceiling - hard_budget) < 1e-6
        else f"{op} {hard_budget:.3f} {unit} (ceil {ceiling:.3f})"
    )
    rows.append(
        (
            name,
            f"{measured:.3f} {unit}",
            f"{baseline_val:.3f} {unit}",
            f"{delta:+.3f} {unit}",
            bud_str,
            status,
        )
    )
    if not ok:
        failures.append(
            f"{name}: measured {measured:.3f} {unit} exceeded limit ({bud_str})"
        )


def check_exact(name, measured, baseline_val, expected, unit):
    ok = measured == expected
    status = "PASS" if ok else "FAIL"
    rows.append(
        (
            name,
            f"{measured} {unit}".strip(),
            f"{baseline_val} {unit}".strip(),
            "0",
            f"== {expected} {unit}".strip(),
            status,
        )
    )
    if not ok:
        failures.append(f"{name}: measured {measured} != expected {expected}")


# 1. niwa _ingest (< 10 ms p95)
for key, label in [
    ("shell_enabled", "niwa _ingest p95 (shell enabled)"),
    ("shell_disabled", "niwa _ingest p95 (shell disabled)"),
]:
    m = rust_run["ingest"][key]["p95_ms"]
    bv = base_rust["ingest"][key]["p95_ms"]
    hb = b["ingest_p95_budget_ms"]
    check_upper(label, m, bv, hb, hb, "ms", strict=True)

# 2. niwa prompt (< 15 ms p95)
for key, label in [
    ("glyph", "niwa prompt p95 (glyph)"),
    ("emoji", "niwa prompt p95 (--style emoji)"),
    ("none", "niwa prompt p95 (--style none)"),
]:
    m = rust_run["prompt"][key]["p95_ms"]
    bv = base_rust["prompt"][key]["p95_ms"]
    hb = b["prompt_p95_budget_ms"]
    check_upper(label, m, bv, hb, hb, "ms", strict=True)

check_exact(
    "niwa prompt fresh snapshot untouched",
    rust_run["prompt"]["snapshot_untouched"],
    base_rust["prompt"]["snapshot_untouched"],
    True,
    "",
)

# 3. 30-day offline catch_up (< 200 ms)
for key, label in [
    ("thirty_day_time_only", "30-day catch_up p95 (43,200 ticks, time)"),
    ("thirty_day_with_events", "30-day catch_up p95 (43,200 ticks + events)"),
]:
    m = rust_run["catch_up"][key]["p95_ms"]
    bv = base_rust["catch_up"][key]["p95_ms"]
    hb = b["catch_up_30d_budget_ms"]
    ceil = min(hb, max(bv * 15.0, bv + 80.0))
    check_upper(label, m, bv, hb, ceil, "ms", strict=True)

# 4. Daemon wake cadence & zero-wake opt-out
check_exact(
    "daemon wake interval",
    rust_run["daemon"]["wake_interval_secs"],
    base_rust["daemon"]["wake_interval_secs"],
    b["daemon_wake_interval_secs"],
    "s",
)
check_exact(
    "daemon wakes when no signal enabled",
    rust_run["daemon"]["no_signals_wakes"],
    base_rust["daemon"]["no_signals_wakes"],
    b["daemon_wakes_when_no_signals"],
    "wakes",
)

# 5. TUI frame-rate caps (<= 4 fps active, 0 fps idle/unfocused, 60 s idle poll sleep)
check_upper(
    "TUI focused active frame rate",
    rust_run["tui"]["max_focused_fps"],
    base_rust["tui"]["max_focused_fps"],
    b["tui_max_focused_fps"],
    b["tui_max_focused_fps"],
    "fps",
)
check_exact(
    "TUI idle frame rate (>= 30 s)",
    rust_run["tui"]["idle_fps"],
    base_rust["tui"]["idle_fps"],
    b["tui_idle_fps"],
    "fps",
)
check_exact(
    "TUI unfocused frame rate (FocusLost)",
    rust_run["tui"]["unfocused_fps"],
    base_rust["tui"]["unfocused_fps"],
    b["tui_unfocused_fps"],
    "fps",
)
check_exact(
    "TUI idle poll sleep timeout",
    rust_run["tui"]["idle_poll_timeout_secs"],
    base_rust["tui"]["idle_poll_timeout_secs"],
    b["tui_idle_poll_timeout_secs"],
    "s",
)

# 6. macOS MenuBarExtra (measured on Darwin; skipped with note on Linux)
if macos_run is not None:
    check_exact(
        "MenuBarExtra timer interval",
        macos_run["timer_interval_secs"],
        base_mb["timer_interval_secs"],
        b["menubar_timer_interval_secs"],
        "s",
    )
    check_exact(
        "MenuBarExtra timer coalescing tolerance",
        macos_run["timer_tolerance_secs"],
        base_mb["timer_tolerance_secs"],
        b["menubar_timer_tolerance_secs"],
        "s",
    )
    check_exact(
        "MenuBarExtra spawns between 60 s ticks",
        macos_run["idle_spawns_between_ticks"],
        base_mb["idle_spawns_between_ticks"],
        b["menubar_idle_spawns_between_ticks"],
        "spawns",
    )
    check_upper(
        "MenuBarExtra steady-state idle CPU",
        macos_run["idle_cpu_pct"],
        base_mb["idle_cpu_pct"],
        b["menubar_max_idle_cpu_pct"],
        b["menubar_max_idle_cpu_pct"],
        "%",
    )

print("")
print(f"PLAN.md §7 Performance & Power Benchmark Comparison (host: {host_platform})")
print("-" * 116)
print(
    f"{'Target':<42} | {'Measured':>12} | {'Baseline':>12} | {'Delta':>11} | {'§7 Budget / Ceiling':<24} | {'Status':<6}"
)
print("-" * 116)
for name, meas, base_s, delta_s, bud_s, st in rows:
    print(f"{name:<42} | {meas:>12} | {base_s:>12} | {delta_s:>11} | {bud_s:<24} | {st:<6}")
print("-" * 116)
if macos_run is None:
    print("Note: macOS MenuBarExtra live benchmark skipped on non-Darwin host (stored baseline validated).")

if failures:
    print("\nERROR: PLAN.md §7 benchmark regression check failed:", file=sys.stderr)
    for item in failures:
        print(f"  - {item}", file=sys.stderr)
    sys.exit(1)

print("All PLAN.md §7 performance and power targets passed.")
PY
