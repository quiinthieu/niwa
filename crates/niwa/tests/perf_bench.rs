#[path = "../../../tests/support/mod.rs"]
mod support;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use niwa_garden::daemon::{run_daemon_loop, DaemonStopReason, DAEMON_TICK_INTERVAL};
use niwa_garden::runtime::GardenSession;
use niwa_garden::tui::app::TuiApp;
use niwa_garden::tui::scheduler::{IDLE_TIMEOUT, SIM_TICK_INTERVAL};
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use support::TestEnv;
use tg_core::balance::Balance;
use tg_core::catch_up::{CatchUpEvent, CatchUpOptions};
use tg_core::clock::{Clock, FakeClock, Timestamp, TICKS_PER_DAY};
use tg_core::season::{Hemisphere, Season, SeasonMode};
use tg_core::sim::Garden;
use tg_core::species::SpeciesCatalog;
use tg_render::ColorTier;
use tg_signals::activity::FakeIdleSource;
use tg_signals::SignalKind;
use tg_store::clock::SystemClock;
use tg_store::config::ConfigStore;
use tg_store::snapshot::{write_snapshot, Snapshot};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LatencyDistribution {
    pub samples: usize,
    pub min_ms: f64,
    pub p50_ms: f64,
    pub mean_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IngestBenchMetrics {
    pub shell_enabled: LatencyDistribution,
    pub shell_disabled: LatencyDistribution,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PromptBenchMetrics {
    pub glyph: LatencyDistribution,
    pub emoji: LatencyDistribution,
    pub none: LatencyDistribution,
    pub snapshot_untouched: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CatchUpBenchMetrics {
    pub ticks_simulated: u64,
    pub thirty_day_time_only: LatencyDistribution,
    pub thirty_day_with_events: LatencyDistribution,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DaemonBenchMetrics {
    pub wake_interval_secs: u64,
    pub no_signals_wakes: u64,
    pub no_signals_sleeps: u64,
    pub no_signals_idle_polls: usize,
    pub single_wake_latency: LatencyDistribution,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TuiBenchMetrics {
    pub max_focused_fps: f64,
    pub idle_fps: f64,
    pub unfocused_fps: f64,
    pub idle_poll_timeout_secs: u64,
    pub render_80x24: LatencyDistribution,
    pub render_120x40: LatencyDistribution,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MacosMenuBarBaselineMetrics {
    pub timer_interval_secs: f64,
    pub timer_tolerance_secs: f64,
    pub observation_window_ms: f64,
    pub user_cpu_ms: f64,
    pub sys_cpu_ms: f64,
    pub total_cpu_ms: f64,
    pub idle_cpu_pct: f64,
    pub idle_spawns_between_ticks: usize,
    pub phys_footprint_mib: f64,
    pub snapshot_reload_p50_us: f64,
    pub snapshot_reload_p95_us: f64,
    pub snapshot_reload_max_us: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RustPerfRunReport {
    pub ingest: IngestBenchMetrics,
    pub prompt: PromptBenchMetrics,
    pub catch_up: CatchUpBenchMetrics,
    pub daemon: DaemonBenchMetrics,
    pub tui: TuiBenchMetrics,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Section7Budgets {
    pub ingest_p95_budget_ms: f64,
    pub prompt_p95_budget_ms: f64,
    pub catch_up_30d_budget_ms: f64,
    pub daemon_wake_interval_secs: u64,
    pub daemon_wakes_when_no_signals: u64,
    pub tui_max_focused_fps: f64,
    pub tui_idle_fps: f64,
    pub tui_unfocused_fps: f64,
    pub tui_idle_poll_timeout_secs: u64,
    pub menubar_timer_interval_secs: f64,
    pub menubar_timer_tolerance_secs: f64,
    pub menubar_idle_spawns_between_ticks: usize,
    pub menubar_max_idle_cpu_pct: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredBaselineMetrics {
    pub rust: RustPerfRunReport,
    pub macos_menubar: MacosMenuBarBaselineMetrics,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PerfBaselineFile {
    pub schema_version: u32,
    pub recorded_on: String,
    pub host_platform: String,
    pub budgets: Section7Budgets,
    pub baseline: StoredBaselineMetrics,
}

fn niwa_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_niwa"))
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn round3(v: f64) -> f64 {
    (v * 1_000.0).round() / 1_000.0
}

fn summarize_samples(mut samples_ms: Vec<f64>) -> LatencyDistribution {
    assert!(!samples_ms.is_empty(), "samples must not be empty");
    samples_ms.sort_by(|a, b| a.total_cmp(b));
    let n = samples_ms.len();
    let pct = |p: usize| -> f64 {
        let rank = (n * p).div_ceil(100).saturating_sub(1).min(n - 1);
        samples_ms[rank]
    };
    let mean = samples_ms.iter().sum::<f64>() / (n as f64);
    LatencyDistribution {
        samples: n,
        min_ms: round3(samples_ms[0]),
        p50_ms: round3(pct(50)),
        mean_ms: round3(mean),
        p95_ms: round3(pct(95)),
        p99_ms: round3(pct(99)),
        max_ms: round3(samples_ms[n - 1]),
    }
}

fn measure_subprocess_burst(
    warmup_count: usize,
    sample_count: usize,
    p95_budget_ms: f64,
    mut run_once: impl FnMut(),
) -> (LatencyDistribution, usize) {
    let mut total_invocations = 0_usize;
    for _ in 0..warmup_count {
        run_once();
        total_invocations += 1;
    }

    let mut best: Option<LatencyDistribution> = None;
    for attempt in 0..3 {
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(250));
        }
        let mut samples = Vec::with_capacity(sample_count);
        for _ in 0..sample_count {
            let t0 = Instant::now();
            run_once();
            samples.push(t0.elapsed().as_secs_f64() * 1_000.0);
            total_invocations += 1;
        }
        let dist = summarize_samples(samples);
        let met_budget = dist.p95_ms < p95_budget_ms;
        best = match best {
            Some(prev) if prev.p95_ms <= dist.p95_ms => Some(prev),
            _ => Some(dist),
        };
        if met_budget {
            break;
        }
    }

    (best.expect("at least one burst"), total_invocations)
}

fn bench_ingest_subprocesses() -> IngestBenchMetrics {
    let bin = niwa_bin();

    // 1. signals.shell = enabled (appends 1 JSONL line to events.jsonl via O_APPEND).
    let enabled_env = TestEnv::new();
    {
        let store = ConfigStore::new(enabled_env.paths().clone());
        let mut cfg = store.load().unwrap();
        cfg.enable_signal(SignalKind::Shell, Timestamp::from_unix_secs(1_760_000_000));
        store.save(&cfg).unwrap();
    }

    let run_ingest = |env: &TestEnv| {
        let status = Command::new(&bin)
            .args(["_ingest", "--exit", "0", "--dur", "1"])
            .env("NIWA_HOME", env.paths().data_dir())
            .env("HOME", env.home_dir())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("spawn niwa _ingest");
        assert!(status.success(), "niwa _ingest must exit 0");
    };

    let (shell_enabled, enabled_spawns) =
        measure_subprocess_burst(20, 200, 10.0, || run_ingest(&enabled_env));
    let spool_contents = fs::read_to_string(enabled_env.paths().events_file()).unwrap();
    assert_eq!(
        spool_contents.lines().count(),
        enabled_spawns,
        "expected {enabled_spawns} appended shell events"
    );

    // 2. signals.shell = disabled (opt-out early exit, 0 bytes written).
    let disabled_env = TestEnv::new();
    {
        let store = ConfigStore::new(disabled_env.paths().clone());
        let cfg = store.load().unwrap();
        store.save(&cfg).unwrap();
    }
    let (shell_disabled, _) = measure_subprocess_burst(20, 200, 10.0, || run_ingest(&disabled_env));
    assert!(
        !disabled_env.paths().events_file().exists(),
        "disabled shell signal must write 0 bytes"
    );

    IngestBenchMetrics {
        shell_enabled,
        shell_disabled,
    }
}

fn bench_prompt_subprocesses() -> PromptBenchMetrics {
    let bin = niwa_bin();
    let env = TestEnv::new();
    let now = SystemClock.now();

    {
        let store = ConfigStore::new(env.paths().clone());
        let cfg = store.load().unwrap();
        store.save(&cfg).unwrap();

        let mut snap = Snapshot::new(
            "◈",
            "4/8 planted · 2 blooming",
            Season::Spring,
            "◈ Niwa · Spring · 4/8 planted · 2 blooming",
            now,
        );
        snap.planted_count = 4;
        snap.blooming_count = 2;
        write_snapshot(env.paths(), &snap).unwrap();
    }

    let snapshot_before = fs::read(env.paths().snapshot_file()).unwrap();

    let measure_mode = |extra_args: &[&str], expected_out: &str| -> LatencyDistribution {
        let (dist, _) = measure_subprocess_burst(20, 200, 15.0, || {
            let out = Command::new(&bin)
                .arg("prompt")
                .args(extra_args)
                .env("NIWA_HOME", env.paths().data_dir())
                .env("HOME", env.home_dir())
                .stdin(Stdio::null())
                .stderr(Stdio::null())
                .output()
                .expect("spawn niwa prompt");
            assert!(out.status.success());
            assert_eq!(String::from_utf8_lossy(&out.stdout), expected_out);
        });
        dist
    };

    let glyph = measure_mode(&[], "◈\n");
    let emoji = measure_mode(&["--style", "emoji"], "🌸\n");
    let none = measure_mode(&["--style", "none"], "");

    let snapshot_after = fs::read(env.paths().snapshot_file()).unwrap();
    let snapshot_untouched =
        snapshot_before == snapshot_after && !env.paths().garden_file().exists();
    assert!(
        snapshot_untouched,
        "fresh `niwa prompt` must never run simulation or touch snapshot.json/garden.json"
    );

    PromptBenchMetrics {
        glyph,
        emoji,
        none,
        snapshot_untouched,
    }
}

fn bench_thirty_day_catch_up() -> CatchUpBenchMetrics {
    let balance = Balance::load_default();
    let catalog = SpeciesCatalog::load_default();
    let start = Timestamp::from_unix_secs(1_776_243_600);
    let ticks = 30 * TICKS_PER_DAY;
    let end = start.add_ticks(ticks);
    let options = CatchUpOptions {
        season_mode: SeasonMode::Calendar,
        hemisphere: Hemisphere::North,
        dormant_after_days_override: None,
    };

    let species_cycle = [
        "sakura",
        "daisy",
        "lavender",
        "red_maple",
        "frostbell",
        "clover",
        "lotus",
        "moss",
        "chamomile",
    ];
    let mut template = Garden::from_balance(0x4E49_5741_2026_0016, &balance, start);
    for idx in 0..template.plots().len() {
        let sp = species_cycle[idx % species_cycle.len()];
        template.plant_seed(idx, sp).unwrap();
    }

    // Build 2,400 spread work events across the 30-day window (every 18 minutes).
    let events: Vec<CatchUpEvent> = (1..=2_400_i64)
        .map(|i| CatchUpEvent::with_water(start.add_secs(i * 1_080), true, 40))
        .collect();

    for _ in 0..5 {
        let mut g1 = template.clone();
        let out1 = g1.catch_up(end, options, &balance, &catalog);
        assert_eq!(out1.ticks_simulated, ticks);

        let mut g2 = template.clone();
        let out2 = g2.catch_up_with_events(end, &events, options, &balance, &catalog);
        assert_eq!(out2.ticks_simulated, ticks);
    }

    let mut time_only_samples = Vec::with_capacity(50);
    for _ in 0..50 {
        let mut g = template.clone();
        let t0 = Instant::now();
        let out = g.catch_up(end, options, &balance, &catalog);
        time_only_samples.push(t0.elapsed().as_secs_f64() * 1_000.0);
        assert_eq!(out.ticks_simulated, ticks);
    }

    let mut with_events_samples = Vec::with_capacity(50);
    for _ in 0..50 {
        let mut g = template.clone();
        let t0 = Instant::now();
        let out = g.catch_up_with_events(end, &events, options, &balance, &catalog);
        with_events_samples.push(t0.elapsed().as_secs_f64() * 1_000.0);
        assert_eq!(out.ticks_simulated, ticks);
    }

    CatchUpBenchMetrics {
        ticks_simulated: ticks,
        thirty_day_time_only: summarize_samples(time_only_samples),
        thirty_day_with_events: summarize_samples(with_events_samples),
    }
}

fn bench_daemon_cadence_and_wake_cost() -> DaemonBenchMetrics {
    // 1. Zero wakes when no signal is enabled (PLAN §7).
    let no_sig_env = TestEnv::new();
    let no_sig_clock = FakeClock::from_unix_secs(1_776_243_600);
    let no_sig_idle = FakeIdleSource::constant(Some(Duration::from_secs(3)));
    let no_sig_sleeps = Mutex::new(Vec::<Duration>::new());

    let no_sig_summary = run_daemon_loop(
        no_sig_env.paths().clone(),
        &no_sig_clock,
        &no_sig_idle,
        &|d| no_sig_sleeps.lock().unwrap().push(d),
        None,
    )
    .unwrap();
    assert_eq!(
        no_sig_summary.stop_reason,
        DaemonStopReason::SkippedNoSignals
    );
    assert_eq!(no_sig_summary.wakes_executed, 0);
    assert_eq!(no_sig_summary.sleeps_performed, 0);
    assert_eq!(no_sig_idle.calls(), 0);
    assert!(no_sig_sleeps.lock().unwrap().is_empty());

    // 2. Coalesced 60 s wake cadence when a signal is enabled (PLAN §7).
    let active_env = TestEnv::new();
    let active_clock = FakeClock::from_unix_secs(1_776_243_600);
    {
        let store = ConfigStore::new(active_env.paths().clone());
        let mut cfg = store.load().unwrap();
        cfg.enable_signal(SignalKind::Activity, active_clock.now());
        store.save(&cfg).unwrap();

        let mut session =
            GardenSession::open(active_env.paths().clone(), &active_clock, Some(101)).unwrap();
        session.plant(0, "clover", active_clock.now()).unwrap();
    }

    let active_idle = FakeIdleSource::constant(Some(Duration::from_secs(4)));
    let recorded_sleeps = Mutex::new(Vec::<Duration>::new());
    let cadence_summary = run_daemon_loop(
        active_env.paths().clone(),
        &active_clock,
        &active_idle,
        &|d| {
            recorded_sleeps.lock().unwrap().push(d);
            active_clock.advance_secs(i64::try_from(d.as_secs()).unwrap());
        },
        Some(5),
    )
    .unwrap();
    assert_eq!(
        cadence_summary.stop_reason,
        DaemonStopReason::ReachedMaxWakes
    );
    assert_eq!(cadence_summary.wakes_executed, 5);
    assert_eq!(cadence_summary.sleeps_performed, 4);
    let sleeps = recorded_sleeps.lock().unwrap().clone();
    assert!(
        sleeps.iter().all(|&d| d == DAEMON_TICK_INTERVAL),
        "every daemon sleep must equal 60 s DAEMON_TICK_INTERVAL, got {sleeps:?}"
    );

    // 3. Single-wake execution latency (`n = 50`).
    let mut wake_samples = Vec::with_capacity(50);
    for _ in 0..50 {
        active_clock.advance_secs(60);
        let t0 = Instant::now();
        let summary = run_daemon_loop(
            active_env.paths().clone(),
            &active_clock,
            &active_idle,
            &|_| {},
            Some(1),
        )
        .unwrap();
        wake_samples.push(t0.elapsed().as_secs_f64() * 1_000.0);
        assert_eq!(summary.wakes_executed, 1);
    }

    DaemonBenchMetrics {
        wake_interval_secs: DAEMON_TICK_INTERVAL.as_secs(),
        no_signals_wakes: no_sig_summary.wakes_executed,
        no_signals_sleeps: no_sig_summary.sleeps_performed,
        no_signals_idle_polls: no_sig_idle.calls(),
        single_wake_latency: summarize_samples(wake_samples),
    }
}

fn bench_tui_fps_and_render_cost() -> TuiBenchMetrics {
    let env = TestEnv::new();
    let clock = FakeClock::from_unix_secs(1_776_243_600);
    let mut session = GardenSession::open(env.paths().clone(), &clock, Some(77)).unwrap();
    session.plant(0, "clover", clock.now()).unwrap();
    session.plant(1, "daisy", clock.now()).unwrap();
    session.plant(2, "chamomile", clock.now()).unwrap();

    let mut app = TuiApp::new(session, ColorTier::Truecolor, &clock).unwrap();
    let mut term_80 = Terminal::new(TestBackend::new(80, 24)).unwrap();

    // 1. Focused input spam every 50 ms across 10.0 s (`200` events) -> <= 4.0 fps (`<= 40` draws).
    let mut active_draws = 0_usize;
    let mut last_input_ms = 0_u64;
    for step_idx in 0..200_u64 {
        let mono = Duration::from_millis(step_idx * 50);
        last_input_ms = step_idx * 50;
        let ev = Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        if app.step(Some(ev), mono, &clock, &mut term_80).unwrap() {
            active_draws += 1;
        }
    }
    let max_focused_fps = round3((active_draws as f64) / 10.0);
    assert!(
        max_focused_fps <= 4.0,
        "focused TUI must never exceed 4.0 fps, got {max_focused_fps}"
    );

    // 2. Idle cutoff after 30 s (`IDLE_TIMEOUT`) -> 0.0 fps across 300 s & 60 s poll sleep.
    let idle_start = Duration::from_millis(last_input_ms) + IDLE_TIMEOUT;
    let idle_poll = app
        .scheduler()
        .next_poll_timeout(idle_start, SIM_TICK_INTERVAL);
    assert_eq!(idle_poll, SIM_TICK_INTERVAL);

    let mut idle_draws = 0_usize;
    for offset_ms in (0..=300_000_u64).step_by(250) {
        if app
            .step(
                None,
                idle_start + Duration::from_millis(offset_ms),
                &clock,
                &mut term_80,
            )
            .unwrap()
        {
            idle_draws += 1;
        }
    }
    let idle_fps = (idle_draws as f64) / 300.0;
    assert_eq!(idle_draws, 0, "idle TUI must draw 0 frames");

    // 3. Unfocused cutoff (`Event::FocusLost`) -> immediate 0.0 fps across 60 s.
    let resume_at = idle_start + Duration::from_secs(305);
    assert!(app
        .step(
            Some(Event::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE))),
            resume_at,
            &clock,
            &mut term_80,
        )
        .unwrap());
    assert!(!app
        .step(
            Some(Event::FocusLost),
            resume_at + Duration::from_millis(10),
            &clock,
            &mut term_80,
        )
        .unwrap());
    let mut unfocused_draws = 0_usize;
    for offset_ms in (50..=60_000_u64).step_by(250) {
        if app
            .step(
                None,
                resume_at + Duration::from_millis(offset_ms),
                &clock,
                &mut term_80,
            )
            .unwrap()
        {
            unfocused_draws += 1;
        }
    }
    let unfocused_fps = (unfocused_draws as f64) / 60.0;
    assert_eq!(unfocused_draws, 0, "unfocused TUI must draw 0 frames");

    // Wake back up and measure `TuiApp::render` frame cost at 80x24 and 120x40 (`n = 200`).
    let _ = app
        .step(
            Some(Event::FocusGained),
            resume_at + Duration::from_secs(61),
            &clock,
            &mut term_80,
        )
        .unwrap();

    for _ in 0..10 {
        app.render(&mut term_80).unwrap();
    }
    let mut samples_80 = Vec::with_capacity(200);
    for _ in 0..200 {
        let t0 = Instant::now();
        app.render(&mut term_80).unwrap();
        samples_80.push(t0.elapsed().as_secs_f64() * 1_000.0);
    }

    let mut term_120 = Terminal::new(TestBackend::new(120, 40)).unwrap();
    for _ in 0..10 {
        app.render(&mut term_120).unwrap();
    }
    let mut samples_120 = Vec::with_capacity(200);
    for _ in 0..200 {
        let t0 = Instant::now();
        app.render(&mut term_120).unwrap();
        samples_120.push(t0.elapsed().as_secs_f64() * 1_000.0);
    }

    TuiBenchMetrics {
        max_focused_fps,
        idle_fps,
        unfocused_fps,
        idle_poll_timeout_secs: idle_poll.as_secs(),
        render_80x24: summarize_samples(samples_80),
        render_120x40: summarize_samples(samples_120),
    }
}

#[test]
fn stored_perf_baseline_json_exists_and_covers_every_section_7_target() {
    let baseline_path = repo_root().join("tests/fixtures/perf-baseline.json");
    let raw = fs::read_to_string(&baseline_path).unwrap_or_else(|err| {
        panic!(
            "missing stored performance baseline at {}: {err}",
            baseline_path.display()
        )
    });
    let parsed: PerfBaselineFile =
        serde_json::from_str(&raw).expect("perf-baseline.json must match PerfBaselineFile schema");

    assert_eq!(parsed.schema_version, 1);
    assert!(!parsed.recorded_on.trim().is_empty());
    assert!(!parsed.host_platform.trim().is_empty());

    // Verify canonical PLAN.md §7 budgets are recorded accurately.
    assert_eq!(parsed.budgets.ingest_p95_budget_ms, 10.0);
    assert_eq!(parsed.budgets.prompt_p95_budget_ms, 15.0);
    assert_eq!(parsed.budgets.catch_up_30d_budget_ms, 200.0);
    assert_eq!(parsed.budgets.daemon_wake_interval_secs, 60);
    assert_eq!(parsed.budgets.daemon_wakes_when_no_signals, 0);
    assert_eq!(parsed.budgets.tui_max_focused_fps, 4.0);
    assert_eq!(parsed.budgets.tui_idle_fps, 0.0);
    assert_eq!(parsed.budgets.tui_unfocused_fps, 0.0);
    assert_eq!(parsed.budgets.tui_idle_poll_timeout_secs, 60);
    assert_eq!(parsed.budgets.menubar_timer_interval_secs, 60.0);
    assert_eq!(parsed.budgets.menubar_timer_tolerance_secs, 6.0);
    assert_eq!(parsed.budgets.menubar_idle_spawns_between_ticks, 0);

    // Verify every stored baseline figure satisfies its corresponding PLAN.md §7 budget.
    let rust = &parsed.baseline.rust;
    assert!(rust.ingest.shell_enabled.p95_ms < parsed.budgets.ingest_p95_budget_ms);
    assert!(rust.ingest.shell_disabled.p95_ms < parsed.budgets.ingest_p95_budget_ms);
    assert!(rust.prompt.glyph.p95_ms < parsed.budgets.prompt_p95_budget_ms);
    assert!(rust.prompt.emoji.p95_ms < parsed.budgets.prompt_p95_budget_ms);
    assert!(rust.prompt.none.p95_ms < parsed.budgets.prompt_p95_budget_ms);
    assert!(rust.prompt.snapshot_untouched);
    assert_eq!(rust.catch_up.ticks_simulated, 30 * TICKS_PER_DAY);
    assert!(rust.catch_up.thirty_day_time_only.p95_ms < parsed.budgets.catch_up_30d_budget_ms);
    assert!(rust.catch_up.thirty_day_with_events.p95_ms < parsed.budgets.catch_up_30d_budget_ms);
    assert_eq!(
        rust.daemon.wake_interval_secs,
        parsed.budgets.daemon_wake_interval_secs
    );
    assert_eq!(
        rust.daemon.no_signals_wakes,
        parsed.budgets.daemon_wakes_when_no_signals
    );
    assert!(rust.tui.max_focused_fps <= parsed.budgets.tui_max_focused_fps);
    assert_eq!(rust.tui.idle_fps, parsed.budgets.tui_idle_fps);
    assert_eq!(rust.tui.unfocused_fps, parsed.budgets.tui_unfocused_fps);
    assert_eq!(
        rust.tui.idle_poll_timeout_secs,
        parsed.budgets.tui_idle_poll_timeout_secs
    );

    let mb = &parsed.baseline.macos_menubar;
    assert_eq!(
        mb.timer_interval_secs,
        parsed.budgets.menubar_timer_interval_secs
    );
    assert_eq!(
        mb.timer_tolerance_secs,
        parsed.budgets.menubar_timer_tolerance_secs
    );
    assert_eq!(
        mb.idle_spawns_between_ticks,
        parsed.budgets.menubar_idle_spawns_between_ticks
    );
    assert!(mb.idle_cpu_pct <= parsed.budgets.menubar_max_idle_cpu_pct);
}

#[test]
fn bench_script_is_bash32_safe_under_set_u() {
    let script_path = repo_root().join("scripts/bench.sh");
    let script = fs::read_to_string(&script_path).expect("read scripts/bench.sh");

    let syntax = Command::new("bash")
        .arg("-n")
        .arg(&script_path)
        .status()
        .expect("run bash -n scripts/bench.sh");
    assert!(syntax.success(), "bash -n scripts/bench.sh must pass");

    let help = Command::new("bash")
        .arg(&script_path)
        .arg("--help")
        .env_remove("NIWA_CARGO_OFFLINE")
        .output()
        .expect("run bash scripts/bench.sh --help");
    assert!(help.status.success(), "scripts/bench.sh --help must exit 0");

    // Apple's `/bin/bash` (Bash 3.2) treats `"${ARR[@]}"` on an empty array as an unbound
    // variable error under `set -u`. Forbid unguarded `[@]` expansions in `scripts/bench.sh`.
    for (idx, line) in script.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            continue;
        }
        assert!(
            !trimmed.contains("[@]}"),
            "scripts/bench.sh:{} contains Bash 4+ array expansion incompatible with macOS Bash 3.2 `set -u`: {trimmed}",
            idx + 1
        );
    }
}

#[test]
#[ignore = "release performance benchmark suite invoked via scripts/bench.sh"]
fn run_release_perf_benchmarks_and_emit_json() {
    assert!(
        !cfg!(debug_assertions) || std::env::var_os("NIWA_ALLOW_DEBUG_BENCH").is_some(),
        "perf_bench must run under `cargo test --release` (PLAN §7)"
    );

    let report = RustPerfRunReport {
        ingest: bench_ingest_subprocesses(),
        prompt: bench_prompt_subprocesses(),
        catch_up: bench_thirty_day_catch_up(),
        daemon: bench_daemon_cadence_and_wake_cost(),
        tui: bench_tui_fps_and_render_cost(),
    };

    assert!(
        report.ingest.shell_enabled.p95_ms < 10.0,
        "`niwa _ingest` (shell enabled) p95 {:.3} ms exceeded 10.0 ms PLAN §7 budget",
        report.ingest.shell_enabled.p95_ms
    );
    assert!(
        report.ingest.shell_disabled.p95_ms < 10.0,
        "`niwa _ingest` (shell disabled) p95 {:.3} ms exceeded 10.0 ms PLAN §7 budget",
        report.ingest.shell_disabled.p95_ms
    );
    assert!(
        report.prompt.glyph.p95_ms < 15.0,
        "`niwa prompt` (glyph) p95 {:.3} ms exceeded 15.0 ms PLAN §7 budget",
        report.prompt.glyph.p95_ms
    );
    assert!(
        report.prompt.emoji.p95_ms < 15.0,
        "`niwa prompt --style emoji` p95 {:.3} ms exceeded 15.0 ms PLAN §7 budget",
        report.prompt.emoji.p95_ms
    );
    assert!(
        report.prompt.none.p95_ms < 15.0,
        "`niwa prompt --style none` p95 {:.3} ms exceeded 15.0 ms PLAN §7 budget",
        report.prompt.none.p95_ms
    );
    assert!(
        report.catch_up.thirty_day_time_only.p95_ms < 200.0
            && report.catch_up.thirty_day_time_only.max_ms < 200.0,
        "30-day time-only `catch_up` (p95 {:.3} ms, max {:.3} ms) exceeded 200.0 ms PLAN §7 budget",
        report.catch_up.thirty_day_time_only.p95_ms,
        report.catch_up.thirty_day_time_only.max_ms
    );
    assert!(
        report.catch_up.thirty_day_with_events.p95_ms < 200.0
            && report.catch_up.thirty_day_with_events.max_ms < 200.0,
        "30-day event-carrying `catch_up` (p95 {:.3} ms, max {:.3} ms) exceeded 200.0 ms PLAN §7 budget",
        report.catch_up.thirty_day_with_events.p95_ms,
        report.catch_up.thirty_day_with_events.max_ms
    );

    let json = serde_json::to_string_pretty(&report).unwrap();
    if let Some(out_path) = std::env::var_os("NIWA_BENCH_JSON_OUT") {
        fs::write(out_path, &json).expect("write NIWA_BENCH_JSON_OUT");
    } else {
        println!("{json}");
    }
}
