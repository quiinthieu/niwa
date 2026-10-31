#[path = "../../../tests/support/mod.rs"]
mod support;

use assert_cmd::Command;
use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use support::TestEnv;
use tg_core::clock::{Clock, FakeClock};
use tg_core::product::{BUNDLE_ID, CLI_BIN};
use tg_signals::activity::FakeIdleSource;
use tg_signals::SignalKind;
use tg_store::config::ConfigStore;
use tg_store::daemon_units::DaemonUnitPaths;
use tg_store::paths::PlatformKind;

fn niwa_cmd(env: &TestEnv) -> Command {
    let mut cmd = Command::cargo_bin(CLI_BIN).expect("resolve compiled niwa binary");
    for (key, value) in env.env_vars() {
        cmd.env(key, value);
    }
    cmd.env("NO_COLOR", "1");
    cmd
}

#[test]
fn daemon_with_fake_idle_source_grants_sun_bonus_on_active_ticks_only() {
    let active_env = TestEnv::new();
    let idle_env = TestEnv::new();
    let time_only_env = TestEnv::new();

    let t0_secs = 1_700_000_000_i64;
    let active_clock = FakeClock::from_unix_secs(t0_secs);
    let idle_clock = FakeClock::from_unix_secs(t0_secs);
    let time_only_clock = FakeClock::from_unix_secs(t0_secs);

    // Opt in to `activity` in both `active_env` and `idle_env`; leave `time_only_env` with no signals.
    for env in [&active_env, &idle_env] {
        let store = ConfigStore::new(env.paths().clone());
        let mut cfg = store.load().unwrap();
        cfg.enable_signal(SignalKind::Activity, active_clock.now());
        store.save(&cfg).unwrap();
    }

    // Seed all three gardens identically with the exact same seed and starter plant at plot 0.
    let starter_id = {
        let mut s_active = niwa_garden::runtime::GardenSession::open(
            active_env.paths().clone(),
            &active_clock,
            Some(777),
        )
        .unwrap();
        let species_id = s_active.save().inventory.keys().next().unwrap().clone();
        s_active.plant(0, &species_id, active_clock.now()).unwrap();

        let mut s_idle = niwa_garden::runtime::GardenSession::open(
            idle_env.paths().clone(),
            &idle_clock,
            Some(777),
        )
        .unwrap();
        s_idle.plant(0, &species_id, idle_clock.now()).unwrap();

        let mut s_time = niwa_garden::runtime::GardenSession::open(
            time_only_env.paths().clone(),
            &time_only_clock,
            Some(777),
        )
        .unwrap();
        s_time.plant(0, &species_id, time_only_clock.now()).unwrap();
        species_id
    };
    let _ = starter_id;

    // Advance 60 s to the end of tick 1, then run `run_daemon_loop` for 5 ticks (5 wakes).
    active_clock.advance_ticks(1);
    idle_clock.advance_ticks(1);
    time_only_clock.advance_ticks(5);

    let active_source = FakeIdleSource::constant(Some(Duration::from_secs(12)));
    let active_sleeps = AtomicU64::new(0);
    let active_summary = niwa_garden::daemon::run_daemon_loop(
        active_env.paths().clone(),
        &active_clock,
        &active_source,
        &|dur: Duration| {
            active_sleeps.fetch_add(1, Ordering::SeqCst);
            active_clock.advance_secs(i64::try_from(dur.as_secs()).unwrap());
        },
        Some(5),
    )
    .expect("run active daemon for 5 wakes");

    assert_eq!(
        active_summary.stop_reason,
        niwa_garden::daemon::DaemonStopReason::ReachedMaxWakes
    );
    assert_eq!(active_summary.wakes_executed, 5);
    assert_eq!(active_summary.sleeps_performed, 4);
    assert_eq!(active_sleeps.load(Ordering::SeqCst), 4);
    assert_eq!(active_summary.ticks_simulated, 5);
    assert_eq!(active_summary.active_samples_recorded, 5);
    assert_eq!(active_summary.idle_samples_recorded, 0);
    assert_eq!(active_source.calls(), 5);

    let idle_source = FakeIdleSource::constant(Some(Duration::from_secs(180)));
    let idle_summary = niwa_garden::daemon::run_daemon_loop(
        idle_env.paths().clone(),
        &idle_clock,
        &idle_source,
        &|dur: Duration| {
            idle_clock.advance_secs(i64::try_from(dur.as_secs()).unwrap());
        },
        Some(5),
    )
    .expect("run idle daemon for 5 wakes");

    assert_eq!(idle_summary.wakes_executed, 5);
    assert_eq!(idle_summary.ticks_simulated, 5);
    assert_eq!(idle_summary.active_samples_recorded, 0);
    assert_eq!(idle_summary.idle_samples_recorded, 5);
    assert_eq!(idle_source.calls(), 5);

    // Inspect final GP across active, idle, and time-only control gardens.
    let s_active =
        niwa_garden::runtime::GardenSession::open(active_env.paths().clone(), &active_clock, None)
            .unwrap();
    let s_idle =
        niwa_garden::runtime::GardenSession::open(idle_env.paths().clone(), &idle_clock, None)
            .unwrap();
    let s_time = niwa_garden::runtime::GardenSession::open(
        time_only_env.paths().clone(),
        &time_only_clock,
        None,
    )
    .unwrap();

    let active_gp = s_active.garden().plots()[0].plant().unwrap().total_gp;
    let idle_gp = s_idle.garden().plots()[0].plant().unwrap().total_gp;
    let time_only_gp = s_time.garden().plots()[0].plant().unwrap().total_gp;

    assert!(
        active_gp > idle_gp,
        "active ticks with FakeIdleSource (< 60s) must earn the sun bonus ({active_gp} > {idle_gp})"
    );
    assert_eq!(
        idle_gp, time_only_gp,
        "idle ticks (>= 60s) must match baseline wall-time growth without sun bonus"
    );
    assert_eq!(s_active.garden().inactive_ticks(), 0);
    assert_eq!(s_idle.garden().inactive_ticks(), 5);
}

#[test]
fn daemon_skips_wake_when_no_signal_is_enabled_and_stops_when_last_signal_disabled() {
    // 1. Brand-new environment with all signals disabled (default):
    //    daemon must exit immediately with 0 wakes, 0 sleeps, 0 idle polls, and 0 files written.
    let env = TestEnv::new();
    let clock = FakeClock::from_unix_secs(1_700_000_000);
    let idle_source = FakeIdleSource::constant(Some(Duration::from_secs(5)));
    let sleep_calls = AtomicU64::new(0);

    let summary = niwa_garden::daemon::run_daemon_loop(
        env.paths().clone(),
        &clock,
        &idle_source,
        &|_dur: Duration| {
            sleep_calls.fetch_add(1, Ordering::SeqCst);
        },
        None,
    )
    .expect("daemon with no signals enabled returns cleanly");

    assert_eq!(
        summary.stop_reason,
        niwa_garden::daemon::DaemonStopReason::SkippedNoSignals
    );
    assert_eq!(summary.wakes_executed, 0);
    assert_eq!(summary.sleeps_performed, 0);
    assert_eq!(sleep_calls.load(Ordering::SeqCst), 0);
    assert_eq!(idle_source.calls(), 0);
    assert!(
        !env.paths().garden_file().exists(),
        "skipped daemon wake must not even initialize or touch garden.json"
    );

    // Also verify the `niwa daemon` CLI binary exits 0 immediately without `--once` when no signal is enabled.
    niwa_cmd(&env).arg("daemon").assert().success();
    assert!(!env.paths().garden_file().exists());

    // 2. Enable `activity`, run an unbounded (`max_wakes = None`) daemon loop, and disable
    //    `activity` during sleep #2: the daemon must exit immediately on wake #3 without polling or sleeping again.
    let config_store = ConfigStore::new(env.paths().clone());
    let mut cfg = config_store.load().unwrap();
    cfg.enable_signal(SignalKind::Activity, clock.now());
    config_store.save(&cfg).unwrap();

    // Initialize save at `t0`, then advance to `t0 + 60`.
    let _ =
        niwa_garden::runtime::GardenSession::open(env.paths().clone(), &clock, Some(99)).unwrap();
    clock.advance_ticks(1);

    let sleeps_mid_run = AtomicU64::new(0);
    let summary_dynamic = niwa_garden::daemon::run_daemon_loop(
        env.paths().clone(),
        &clock,
        &idle_source,
        &|dur: Duration| {
            let count = sleeps_mid_run.fetch_add(1, Ordering::SeqCst) + 1;
            clock.advance_secs(i64::try_from(dur.as_secs()).unwrap());
            if count == 2 {
                let mut latest = config_store.load().unwrap();
                latest.disable_signal(SignalKind::Activity, false);
                config_store.save(&latest).unwrap();
            }
        },
        None,
    )
    .expect("daemon stops cleanly as soon as last signal is disabled");

    assert_eq!(
        summary_dynamic.stop_reason,
        niwa_garden::daemon::DaemonStopReason::SkippedNoSignals
    );
    assert_eq!(summary_dynamic.wakes_executed, 2);
    assert_eq!(summary_dynamic.sleeps_performed, 2);
    assert_eq!(idle_source.calls(), 2);
}

#[test]
fn cli_daemon_install_status_and_uninstall_round_trip_in_temp_home() {
    let env = TestEnv::new();

    let expected_unit_path = match PlatformKind::current() {
        PlatformKind::MacOs => env
            .home_dir()
            .join("Library")
            .join("LaunchAgents")
            .join(format!("{BUNDLE_ID}.daemon.plist")),
        PlatformKind::Linux => env
            .xdg_config_home()
            .join("systemd")
            .join("user")
            .join(format!("{CLI_BIN}.service")),
    };

    assert!(!expected_unit_path.exists());

    // Status before install.
    let status_before = niwa_cmd(&env)
        .args(["daemon", "status"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let status_before_str = String::from_utf8(status_before).unwrap();
    assert!(status_before_str.contains("not installed"));

    // Install user daemon unit inside `TestEnv`.
    let install_out = niwa_cmd(&env)
        .args(["daemon", "install"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let install_str = String::from_utf8(install_out).unwrap();
    assert!(install_str.contains(&expected_unit_path.display().to_string()));
    assert!(expected_unit_path.is_file());

    let unit_contents = fs::read_to_string(&expected_unit_path).unwrap();
    assert!(unit_contents.contains("daemon"));

    // Also verify `DaemonUnitPaths::resolve_from_env` agrees with the installed path.
    let resolved = DaemonUnitPaths::resolve_from_env(
        PlatformKind::current(),
        |key| {
            env.env_vars()
                .into_iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.as_os_str().to_os_string())
        },
        None,
    )
    .unwrap();
    assert_eq!(resolved.unit_path(), expected_unit_path);
    assert!(resolved.is_installed());

    // Status after install.
    let status_after = niwa_cmd(&env)
        .args(["daemon", "status"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let status_after_str = String::from_utf8(status_after).unwrap();
    assert!(status_after_str.contains("installed"));

    // Uninstall removes the unit file cleanly from temp home.
    let uninstall_out = niwa_cmd(&env)
        .args(["daemon", "uninstall"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let uninstall_str = String::from_utf8(uninstall_out).unwrap();
    assert!(uninstall_str.contains(&expected_unit_path.display().to_string()));
    assert!(!expected_unit_path.exists());

    // Second uninstall is idempotent and exits 0.
    niwa_cmd(&env)
        .args(["daemon", "uninstall"])
        .assert()
        .success();
}
