#[path = "../../../tests/support/mod.rs"]
mod support;

use assert_cmd::Command;
use std::fs;
use std::io::Write;
use support::TestEnv;
use tg_core::clock::{Clock, FakeClock};
use tg_signals::consent::ConsentCatalog;
use tg_signals::{DurationBucket, EventSpool, ShellExitClass, SignalKind, WorkEvent};
use tg_store::config::ConfigStore;

fn niwa_cmd(env: &TestEnv) -> Command {
    let mut cmd = Command::cargo_bin("niwa").expect("niwa binary built");
    cmd.envs(env.env_vars());
    cmd.env("NO_COLOR", "1");
    cmd
}

#[test]
fn signals_list_defaults_to_time_always_on_and_work_signals_disabled() {
    let env = TestEnv::new();

    for args in [&["signals"][..], &["signals", "list"][..]] {
        let assert = niwa_cmd(&env).args(args).assert().success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        assert!(stdout.contains("time"));
        assert!(stdout.contains("always on"));
        assert!(stdout.contains("activity"));
        assert!(stdout.contains("git"));
        assert!(stdout.contains("shell"));
        assert!(stdout.contains("disabled"));
    }
}

#[test]
fn signals_enable_prints_shared_disclosure_and_waits_for_explicit_y() {
    let env = TestEnv::new();
    let config_store = ConfigStore::new(env.paths().clone());
    let shared_shell = ConsentCatalog::load_default()
        .signal(SignalKind::Shell)
        .clone();

    // 1. Declining with `n` prints the shared disclosure and leaves the signal disabled.
    let declined = niwa_cmd(&env)
        .args(["signals", "enable", "shell"])
        .write_stdin("n\n")
        .assert()
        .success();
    let declined_out = String::from_utf8(declined.get_output().stdout.clone()).unwrap();
    assert!(declined_out.contains(&shared_shell.summary));
    for stored_item in &shared_shell.stores {
        assert!(declined_out.contains(stored_item));
    }
    for never_item in &shared_shell.never_stores {
        assert!(declined_out.contains(never_item));
    }

    let cfg_after_decline = config_store.load().unwrap();
    assert!(!cfg_after_decline.signals.shell.enabled);
    assert_eq!(cfg_after_decline.signals.shell.consented_at, None);

    // 2. Confirming with `y` records `enabled = true` and a `consented_at` timestamp in `config.toml`.
    let accepted = niwa_cmd(&env)
        .args(["signals", "enable", "shell"])
        .write_stdin("y\n")
        .assert()
        .success();
    let accepted_out = String::from_utf8(accepted.get_output().stdout.clone()).unwrap();
    assert!(accepted_out.contains(&shared_shell.summary));
    assert!(accepted_out.contains("Enabled signal 'shell'"));

    let cfg_after_accept = config_store.load().unwrap();
    assert!(cfg_after_accept.signals.shell.enabled);
    assert!(cfg_after_accept.signals.shell.consented_at.is_some());

    // 3. `time` and unknown signal names fail with clear errors.
    niwa_cmd(&env)
        .args(["signals", "enable", "time"])
        .write_stdin("y\n")
        .assert()
        .failure();
    niwa_cmd(&env)
        .args(["signals", "enable", "telepathy"])
        .write_stdin("y\n")
        .assert()
        .failure();
}

#[test]
fn signals_disable_and_purge_manage_consent_and_delete_events() {
    let env = TestEnv::new();
    let config_store = ConfigStore::new(env.paths().clone());
    let spool = EventSpool::new(env.paths().clone());

    niwa_cmd(&env)
        .args(["signals", "enable", "shell", "--yes"])
        .assert()
        .success();
    niwa_cmd(&env)
        .args(["signals", "enable", "git", "--yes"])
        .assert()
        .success();

    let _ = env.stage_spool_fixture("sample_events.jsonl");
    assert_eq!(spool.read().unwrap().events.len(), 3);

    // Disable `shell` WITHOUT `--purge`: stops collection, keeps `consented_at` and spool file.
    niwa_cmd(&env)
        .args(["signals", "disable", "shell"])
        .assert()
        .success();
    let cfg_paused = config_store.load().unwrap();
    assert!(!cfg_paused.signals.shell.enabled);
    assert!(cfg_paused.signals.shell.consented_at.is_some());
    assert_eq!(spool.read().unwrap().events.len(), 3);

    // Disable `shell` WITH `--purge`: clears `consented_at` and deletes `shell` events from spool.
    niwa_cmd(&env)
        .args(["signals", "disable", "shell", "--purge"])
        .assert()
        .success();
    let cfg_purged = config_store.load().unwrap();
    assert!(!cfg_purged.signals.shell.enabled);
    assert_eq!(cfg_purged.signals.shell.consented_at, None);
    let remaining = spool.read().unwrap().events;
    assert_eq!(remaining.len(), 2);
    assert!(remaining
        .iter()
        .all(|ev| ev.signal_kind() != SignalKind::Shell));

    // Purge `git` and `activity`: `events.jsonl` is deleted completely once empty.
    niwa_cmd(&env)
        .args(["signals", "disable", "git", "--purge"])
        .assert()
        .success();
    niwa_cmd(&env)
        .args(["signals", "disable", "activity", "--purge"])
        .assert()
        .success();
    assert!(!env.paths().events_file().exists());
}

#[test]
fn runtime_applies_consented_spool_events_updates_diagnostics_and_compacts_spool() {
    let env = TestEnv::new();
    let control_env = TestEnv::new();

    let t_consent = FakeClock::from_unix_secs(1_700_000_000);
    // Consent to `shell` and `git` at `t_consent`, leaving `activity` disabled.
    let mut cfg = ConfigStore::new(env.paths().clone()).load().unwrap();
    cfg.enable_signal(SignalKind::Shell, t_consent.now());
    cfg.enable_signal(SignalKind::Git, t_consent.now());
    ConfigStore::new(env.paths().clone()).save(&cfg).unwrap();

    // Create both experimental and time-only control gardens 600s later (`t_create = 1_700_000_600`).
    let clock = FakeClock::from_unix_secs(1_700_000_600);
    let t_create = clock.now();
    {
        let mut session =
            niwa_garden::runtime::GardenSession::open(env.paths().clone(), &clock, Some(123))
                .expect("open experimental garden");
        let starter_id = session.save().inventory.keys().next().unwrap().clone();
        session.plant(0, &starter_id, t_create).unwrap();

        let mut control = niwa_garden::runtime::GardenSession::open(
            control_env.paths().clone(),
            &clock,
            Some(123),
        )
        .expect("open control garden");
        control.plant(0, &starter_id, t_create).unwrap();
    }

    // Populate `events.jsonl` with:
    // - 1 valid consented `shell` event in tick 0 (`t_create + 30`)
    // - 1 valid consented `git_commit` event in tick 1 (`t_create + 120`)
    // - 1 unconsented `activity` event (`t_create + 150`, must be ignored)
    // - 1 consented but pre-creation `git_commit` (`t_create - 120 >= t_consent`, counts in diagnostics)
    // - 1 consented but future `git_commit` (`t_create + 900`, counts in diagnostics)
    // - 1 corrupt JSON line (counts in `corrupt_spool_lines`)
    // - 1 valid consented `shell` event in an incomplete partial tick (`t_create + 195` when `now = t_create + 200`)
    let partial_tick_ev = WorkEvent::Shell {
        ts: t_create.add_secs(195),
        status: ShellExitClass::Ok,
        dur_bucket: DurationBucket::Long,
    };
    {
        let mut f = fs::File::create(env.paths().events_file()).unwrap();
        writeln!(
            f,
            "{}",
            serde_json::to_string(&WorkEvent::Shell {
                ts: t_create.add_secs(30),
                status: ShellExitClass::Ok,
                dur_bucket: DurationBucket::Medium,
            })
            .unwrap()
        )
        .unwrap();
        writeln!(
            f,
            "{}",
            serde_json::to_string(&WorkEvent::GitCommit {
                ts: t_create.add_secs(120),
            })
            .unwrap()
        )
        .unwrap();
        writeln!(
            f,
            "{}",
            serde_json::to_string(&WorkEvent::Activity {
                ts: t_create.add_secs(150),
                active: true,
            })
            .unwrap()
        )
        .unwrap();
        writeln!(
            f,
            "{}",
            serde_json::to_string(&WorkEvent::GitCommit {
                ts: t_create.add_secs(-120),
            })
            .unwrap()
        )
        .unwrap();
        writeln!(
            f,
            "{}",
            serde_json::to_string(&WorkEvent::GitCommit {
                ts: t_create.add_secs(900),
            })
            .unwrap()
        )
        .unwrap();
        writeln!(f, "{{corrupt line in spool").unwrap();
        writeln!(f, "{}", serde_json::to_string(&partial_tick_ev).unwrap()).unwrap();
    }

    // Advance 200s (3 completed 60s ticks + 20s partial remainder).
    clock.advance_secs(200);
    let exp_session =
        niwa_garden::runtime::GardenSession::open(env.paths().clone(), &clock, None).unwrap();
    let ctrl_session =
        niwa_garden::runtime::GardenSession::open(control_env.paths().clone(), &clock, None)
            .unwrap();

    let exp_report = exp_session.status_report();
    let ctrl_report = ctrl_session.status_report();

    let exp_gp = exp_report.plots[0].plant.as_ref().unwrap().total_gp;
    let ctrl_gp = ctrl_report.plots[0].plant.as_ref().unwrap().total_gp;
    assert!(
        exp_gp > ctrl_gp,
        "consented shell + git events must boost plant GP ({exp_gp}) above time-only control ({ctrl_gp})"
    );
    assert_eq!(exp_report.diagnostics.corrupt_spool_lines, 1);
    assert_eq!(exp_report.diagnostics.dropped_pre_creation_events, 1);
    assert_eq!(exp_report.diagnostics.dropped_future_events, 1);
    // Tick 0 (`shell`) was active (0), Tick 1 (`git_commit`, water-only) -> 1, and Tick 2 had
    // only an unconsented `activity` event (ignored, not active) -> `inactive_ticks == 2`.
    assert_eq!(exp_report.inactive_ticks, 2);

    // Compaction kept ONLY the partial-tick event (`t_create + 195`) and removed everything else.
    let after_first_compact = EventSpool::new(env.paths().clone()).read().unwrap();
    assert_eq!(after_first_compact.corrupt_lines, 0);
    assert_eq!(after_first_compact.events, vec![partial_tick_ev]);

    // Advancing 40s completes tick 3 (`t_create + 240`), applying `partial_tick_ev` and removing `events.jsonl`.
    clock.advance_secs(40);
    let exp_after_tick4 =
        niwa_garden::runtime::GardenSession::open(env.paths().clone(), &clock, None).unwrap();
    assert_eq!(exp_after_tick4.status_report().inactive_ticks, 0);
    assert!(!env.paths().events_file().exists());
}
