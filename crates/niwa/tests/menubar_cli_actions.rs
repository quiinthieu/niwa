#[path = "../../../tests/support/mod.rs"]
mod support;

use assert_cmd::Command;
use clap::Parser;
use niwa_garden::cli::{execute_with, Cli};
use niwa_garden::runtime::GardenSession;
use support::TestEnv;
use tg_core::clock::{Clock, FakeClock, Timestamp};
use tg_core::model::Stage;
use tg_core::season::Season;
use tg_core::sim::TickInput;
use tg_store::save::{GardenSave, GardenStore};
use tg_store::snapshot::read_snapshot;
use tg_store::spool::EventSpool;

fn niwa_cmd(env: &TestEnv) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_niwa"));
    cmd.envs(env.env_vars());
    cmd
}

#[test]
fn harvest_all_cli_harvests_every_blooming_plot_leaves_unbloomed_plots_and_updates_snapshot() {
    let env = TestEnv::new();
    let t0 = Timestamp::from_unix_secs(1_745_000_000);
    let clock = FakeClock::new(t0);

    // Initialize garden, plant plot 0 and advance it to Blooming, then plant a fresh Seed in plot 2.
    {
        let mut session = GardenSession::open(env.paths().clone(), &clock, Some(77)).unwrap();
        let _ = session.plant(0, "moss", clock.now()).unwrap();
        let balance = session.balance().clone();
        let catalog = session.catalog().clone();

        let mut save = session.save().clone();
        let mut garden = save.to_garden();
        while garden.plots()[0].plant().unwrap().stage != Stage::Blooming {
            garden.tick_with_catalog(TickInput::active(), Season::Spring, &balance, &catalog);
        }
        clock.set(garden.last_tick_at());
        garden.plant_seed(2, "moss").unwrap();
        let updated_plots = GardenSave::from_garden(&garden);
        save.plots = updated_plots.plots;
        save.tick_index = updated_plots.tick_index;
        save.last_tick_at = updated_plots.last_tick_at;
        GardenStore::new(env.paths().clone()).save(&save).unwrap();
    }

    // Refresh snapshot before harvest and confirm 1 blooming + 2 planted.
    let _ = GardenSession::open(env.paths().clone(), &clock, None).unwrap();
    let before_snap = read_snapshot(env.paths()).unwrap().unwrap();
    assert_eq!(before_snap.planted_count, 2);
    assert_eq!(before_snap.blooming_count, 1);
    assert_eq!(before_snap.glyph, "◈");

    // Run `niwa harvest --all` via `execute_with` (deterministic clock).
    let cli = Cli::parse_from(["niwa", "harvest", "--all"]);
    execute_with(cli, env.paths().clone(), &clock).expect("niwa harvest --all");

    let after_snap = read_snapshot(env.paths()).unwrap().unwrap();
    assert_eq!(
        after_snap.planted_count, 1,
        "blooming plot 0 harvested while seed plot 2 remains"
    );
    assert_eq!(after_snap.blooming_count, 0);
    assert_eq!(after_snap.glyph, "·");

    let reloaded = GardenStore::new(env.paths().clone())
        .load()
        .unwrap()
        .unwrap()
        .save;
    assert!(reloaded.plots[0].plant().is_none());
    assert_eq!(reloaded.plots[2].plant().unwrap().stage, Stage::Seed);
    assert_eq!(reloaded.almanac["moss"].harvest_count, 1);

    // Running `niwa harvest --all` as a real subprocess when 0 plots are blooming succeeds cleanly.
    let empty_harvest = niwa_cmd(&env).args(["harvest", "--all"]).assert().success();
    let stdout = String::from_utf8(empty_harvest.get_output().stdout.clone()).unwrap();
    assert!(
        stdout.contains("No blooming plants"),
        "expected friendly zero-bloom message, got:\n{stdout}"
    );
}

#[test]
fn signals_pause_and_resume_via_config_and_subcommands_gate_collection_without_losing_consent() {
    let env = TestEnv::new();

    // Consent to `activity` and `shell`, leaving `git` disabled.
    niwa_cmd(&env)
        .args(["signals", "enable", "activity", "--yes"])
        .assert()
        .success();
    niwa_cmd(&env)
        .args(["signals", "enable", "shell", "--yes"])
        .assert()
        .success();

    let activity_consent_before = String::from_utf8(
        niwa_cmd(&env)
            .args(["config", "get", "signals.activity.consented_at"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone(),
    )
    .unwrap();
    assert_ne!(activity_consent_before.trim(), "none");

    // Pause via `niwa config set signals.paused true` (used by the macOS menu bar companion).
    niwa_cmd(&env)
        .args(["config", "set", "signals.paused", "true"])
        .assert()
        .success();
    let paused_val = String::from_utf8(
        niwa_cmd(&env)
            .args(["config", "get", "signals.paused"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone(),
    )
    .unwrap();
    assert_eq!(paused_val.trim(), "true");

    // While paused, `_ingest` records zero events and `daemon --once` skips waking.
    niwa_cmd(&env)
        .args(["_ingest", "--exit", "0", "--dur", "0.4"])
        .assert()
        .success();
    let spool = EventSpool::new(env.paths().clone());
    assert!(
        spool.read().unwrap().events.is_empty(),
        "paused signals must not append shell events"
    );

    // Individual consent records remain intact while paused!
    let activity_consent_paused = String::from_utf8(
        niwa_cmd(&env)
            .args(["config", "get", "signals.activity.consented_at"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone(),
    )
    .unwrap();
    assert_eq!(activity_consent_paused, activity_consent_before);

    // Resume via `niwa signals resume` (and test `niwa signals pause` CLI subcommand too).
    niwa_cmd(&env)
        .args(["signals", "resume"])
        .assert()
        .success();
    niwa_cmd(&env)
        .args(["_ingest", "--exit", "0", "--dur", "0.4"])
        .assert()
        .success();
    assert_eq!(
        spool.read().unwrap().events.len(),
        1,
        "resuming signals restores shell event collection without re-prompting"
    );

    niwa_cmd(&env).args(["signals", "pause"]).assert().success();
    let paused_again = String::from_utf8(
        niwa_cmd(&env)
            .args(["config", "get", "signals.paused"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone(),
    )
    .unwrap();
    assert_eq!(paused_again.trim(), "true");
}

#[test]
fn daemon_once_with_injected_idle_secs_records_activity_and_advances_snapshot() {
    let env = TestEnv::new();
    let t0 = Timestamp::from_unix_secs(1_745_000_000);
    let clock = FakeClock::new(t0);

    // Initialize garden, plant a seed, and enable `activity` at `t0`.
    {
        let mut session = GardenSession::open(env.paths().clone(), &clock, Some(99)).unwrap();
        let _ = session.plant(0, "moss", clock.now()).unwrap();
    }
    execute_with(
        Cli::parse_from(["niwa", "signals", "enable", "activity", "--yes"]),
        env.paths().clone(),
        &clock,
    )
    .unwrap();

    let gp_t0 = GardenStore::new(env.paths().clone())
        .load()
        .unwrap()
        .unwrap()
        .save
        .plots[0]
        .plant()
        .unwrap()
        .total_gp;

    // 1. Advance 60 s and inject `--idle-secs 12.5` (< 60 s -> active tick with `sun` bonus!).
    clock.advance_secs(60);
    execute_with(
        Cli::parse_from(["niwa", "daemon", "--once", "--idle-secs", "12.5"]),
        env.paths().clone(),
        &clock,
    )
    .unwrap();

    let save_active = GardenStore::new(env.paths().clone())
        .load()
        .unwrap()
        .unwrap()
        .save;
    let gp_after_active = save_active.plots[0].plant().unwrap().total_gp;
    let active_gain = gp_after_active - gp_t0;
    assert!(active_gain > 0);
    assert_eq!(save_active.inactive_ticks, 0);
    assert_eq!(
        read_snapshot(env.paths()).unwrap().unwrap().updated_at,
        Timestamp::from_unix_secs(t0.as_unix_secs() + 60)
    );

    // 2. Advance another 60 s and inject `--idle-secs 180.0` (>= 60 s -> passive idle tick).
    clock.advance_secs(60);
    execute_with(
        Cli::parse_from(["niwa", "daemon", "--once", "--idle-secs", "180.0"]),
        env.paths().clone(),
        &clock,
    )
    .unwrap();

    let save_idle = GardenStore::new(env.paths().clone())
        .load()
        .unwrap()
        .unwrap()
        .save;
    let idle_gain = save_idle.plots[0].plant().unwrap().total_gp - gp_after_active;
    assert!(
        active_gain > idle_gain,
        "active tick gain ({active_gain}) must exceed passive tick gain ({idle_gain})"
    );
    assert_eq!(save_idle.inactive_ticks, 1);

    // 3. Reject negative `--idle-secs`.
    let bad = execute_with(
        Cli::parse_from(["niwa", "daemon", "--once", "--idle-secs", "-5.0"]),
        env.paths().clone(),
        &clock,
    );
    assert!(bad.is_err(), "negative --idle-secs must be rejected");
}
