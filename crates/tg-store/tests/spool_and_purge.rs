#[path = "../../../tests/support/mod.rs"]
mod support;

use std::fs;
use std::io::Write;
use std::str::FromStr;
use support::TestEnv;
use tg_core::balance::Balance;
use tg_core::clock::Timestamp;
use tg_core::sim::Garden;
use tg_store::config::SignalKind;
use tg_store::save::{GardenSave, GardenStore, GitPollState, WaterBudgetState};
use tg_store::spool::{DurationBucket, EventSpool, ShellExitClass, WorkEvent};

#[test]
fn sample_events_fixture_round_trips_byte_for_byte() {
    let env = TestEnv::new();
    let _ = env.stage_spool_fixture("sample_events.jsonl");

    let spool = EventSpool::new(env.paths().clone());
    let outcome = spool.read().expect("read staged sample_events.jsonl");

    assert_eq!(outcome.corrupt_lines, 0);
    assert_eq!(
        outcome.events,
        vec![
            WorkEvent::Shell {
                ts: Timestamp::from_unix_secs(1_700_000_060),
                status: ShellExitClass::Ok,
                dur_bucket: DurationBucket::Fast,
            },
            WorkEvent::GitCommit {
                ts: Timestamp::from_unix_secs(1_700_000_120),
            },
            WorkEvent::Activity {
                ts: Timestamp::from_unix_secs(1_700_000_180),
                active: true,
            },
        ]
    );

    let reserialized = outcome
        .events
        .iter()
        .map(|ev| serde_json::to_string(ev).expect("serialize WorkEvent"))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    let expected = env.read_spool_fixture("sample_events.jsonl");
    assert_eq!(reserialized, expected);
}

#[test]
fn work_event_rejects_unknown_or_privacy_violating_fields() {
    let forbidden_payloads = [
        r#"{"ts":1700000060,"kind":"shell","status":"ok","dur_bucket":"fast","cmd":"git push"}"#,
        r#"{"ts":1700000060,"kind":"shell","status":"ok","dur_bucket":"fast","cwd":"/Users/alice/secret"}"#,
        r#"{"ts":1700000120,"kind":"git_commit","repo":"secret-repo"}"#,
        r#"{"ts":1700000120,"kind":"git_commit","message":"fix secret bug"}"#,
        r#"{"ts":1700000180,"kind":"activity","active":true,"window":"Private Browser"}"#,
        r#"{"ts":1700000180,"kind":"unknown_signal"}"#,
    ];

    for payload in forbidden_payloads {
        let parsed = serde_json::from_str::<WorkEvent>(payload);
        assert!(
            parsed.is_err(),
            "WorkEvent must reject payload with unknown or forbidden fields: {payload}"
        );
    }
}

#[test]
fn duration_bucket_and_exit_class_helpers_classify_boundaries() {
    assert_eq!(ShellExitClass::from_exit_code(0), ShellExitClass::Ok);
    assert_eq!(ShellExitClass::from_exit_code(1), ShellExitClass::Err);
    assert_eq!(ShellExitClass::from_exit_code(130), ShellExitClass::Err);
    assert_eq!(ShellExitClass::from_exit_code(-1), ShellExitClass::Err);

    assert_eq!(
        DurationBucket::from_duration_secs(-0.5),
        DurationBucket::Instant
    );
    assert_eq!(
        DurationBucket::from_duration_secs(f64::NAN),
        DurationBucket::Instant
    );
    assert_eq!(
        DurationBucket::from_duration_secs(0.0),
        DurationBucket::Instant
    );
    assert_eq!(
        DurationBucket::from_duration_secs(0.999),
        DurationBucket::Instant
    );
    assert_eq!(
        DurationBucket::from_duration_secs(1.0),
        DurationBucket::Fast
    );
    assert_eq!(
        DurationBucket::from_duration_secs(9.999),
        DurationBucket::Fast
    );
    assert_eq!(
        DurationBucket::from_duration_secs(10.0),
        DurationBucket::Medium
    );
    assert_eq!(
        DurationBucket::from_duration_secs(59.999),
        DurationBucket::Medium
    );
    assert_eq!(
        DurationBucket::from_duration_secs(60.0),
        DurationBucket::Long
    );
    assert_eq!(
        DurationBucket::from_duration_secs(3_600.0),
        DurationBucket::Long
    );
}

#[test]
fn spool_skips_and_counts_corrupt_or_non_utf8_lines() {
    let env = TestEnv::new();
    let spool = EventSpool::new(env.paths().clone());

    // Missing spool file reads cleanly as empty.
    let initial = spool.read().expect("read missing spool");
    assert!(initial.events.is_empty());
    assert_eq!(initial.corrupt_lines, 0);

    let events_path = env.paths().events_file();
    let mut raw: Vec<u8> = Vec::new();
    writeln!(
        raw,
        r#"{{"ts":1700000060,"kind":"shell","status":"ok","dur_bucket":"instant"}}"#
    )
    .unwrap();
    writeln!(raw, "   ").unwrap(); // Blank whitespace line is ignored, not corrupt.
    writeln!(raw, "{{truncated json").unwrap(); // Corrupt #1
    raw.extend_from_slice(&[0xFF, 0xFE, b'\n']); // Invalid UTF-8: Corrupt #2
    writeln!(
        raw,
        r#"{{"ts":1700000090,"kind":"shell","status":"ok","dur_bucket":"fast","cmd":"ls"}}"#
    )
    .unwrap(); // Forbidden extra field: Corrupt #3
    writeln!(raw, r#"{{"ts":1700000120,"kind":"git_commit"}}"#).unwrap();
    fs::write(&events_path, raw).expect("write mixed spool");

    let read = spool.read().expect("read mixed spool");
    assert_eq!(read.corrupt_lines, 3);
    assert_eq!(
        read.events,
        vec![
            WorkEvent::Shell {
                ts: Timestamp::from_unix_secs(1_700_000_060),
                status: ShellExitClass::Ok,
                dur_bucket: DurationBucket::Instant,
            },
            WorkEvent::GitCommit {
                ts: Timestamp::from_unix_secs(1_700_000_120),
            },
        ]
    );

    let counts = spool.count_by_signal().expect("count by signal");
    assert_eq!(counts.activity, 0);
    assert_eq!(counts.git, 1);
    assert_eq!(counts.shell, 1);
    assert_eq!(counts.corrupt_lines, 3);
    assert_eq!(counts.total_valid(), 2);
}

#[test]
fn spool_compact_removes_applied_events_and_preserves_concurrent_tail_appends() {
    let env = TestEnv::new();
    let spool = EventSpool::new(env.paths().clone());

    let ev1 = WorkEvent::GitCommit {
        ts: Timestamp::from_unix_secs(1_700_000_060),
    };
    let ev2 = WorkEvent::Activity {
        ts: Timestamp::from_unix_secs(1_700_000_095),
        active: true,
    };
    let ev_concurrent = WorkEvent::Shell {
        ts: Timestamp::from_unix_secs(1_700_000_100),
        status: ShellExitClass::Err,
        dur_bucket: DurationBucket::Medium,
    };

    spool.append(ev1).expect("append ev1");
    spool.append(ev2).expect("append ev2");

    let read = spool.read().expect("read before concurrent append");
    assert_eq!(read.events.len(), 2);

    // Simulate a lock-free `_ingest` appending `ev_concurrent` before `compact` finishes.
    spool
        .append(ev_concurrent)
        .expect("concurrent append after read");

    // Retain `ev2` (in-progress partial tick) while compacting `ev1`.
    spool
        .compact(read.bytes_read, &[ev2])
        .expect("compact preserving concurrent tail");

    let after = spool.read().expect("read after compact");
    assert_eq!(after.corrupt_lines, 0);
    assert_eq!(after.events, vec![ev2, ev_concurrent]);

    // Compacting everything when no concurrent tail arrived deletes `events.jsonl`.
    spool
        .compact(after.bytes_read, &[])
        .expect("compact all remaining events");
    assert!(!env.paths().events_file().exists());
}

#[test]
fn purge_signal_removes_target_events_and_deletes_empty_spool() {
    let env = TestEnv::new();
    let spool = EventSpool::new(env.paths().clone());

    let shell_ev1 = WorkEvent::Shell {
        ts: Timestamp::from_unix_secs(1_700_000_060),
        status: ShellExitClass::Ok,
        dur_bucket: DurationBucket::Fast,
    };
    let git_ev = WorkEvent::GitCommit {
        ts: Timestamp::from_unix_secs(1_700_000_120),
    };
    let shell_ev2 = WorkEvent::Shell {
        ts: Timestamp::from_unix_secs(1_700_000_180),
        status: ShellExitClass::Err,
        dur_bucket: DurationBucket::Long,
    };

    spool.append(shell_ev1).unwrap();
    spool.append(git_ev).unwrap();
    spool.append(shell_ev2).unwrap();

    let purged_shell = spool.purge_signal(SignalKind::Shell).expect("purge shell");
    assert_eq!(purged_shell.removed_events, 2);
    assert_eq!(purged_shell.remaining_events, 1);
    assert!(env.paths().events_file().exists());

    let remaining = spool.read().unwrap();
    assert_eq!(remaining.events, vec![git_ev]);

    let purged_git = spool.purge_signal(SignalKind::Git).expect("purge git");
    assert_eq!(purged_git.removed_events, 1);
    assert_eq!(purged_git.remaining_events, 0);
    assert!(
        !env.paths().events_file().exists(),
        "events.jsonl must be deleted when no events remain after purge"
    );
}

#[test]
fn signal_kind_parsing_and_water_budget_persistence_round_trip() {
    for kind in SignalKind::ALL {
        let parsed = SignalKind::from_str(kind.as_str()).expect("parse SignalKind");
        assert_eq!(parsed, kind);
        assert_eq!(kind.to_string(), kind.as_str());
    }
    assert!(SignalKind::from_str("time").is_err());
    assert!(SignalKind::from_str("unknown").is_err());

    let env = TestEnv::new();
    let store = GardenStore::new(env.paths().clone());

    // Existing v1 save fixture without `water_budget` loads with default empty budget.
    let _ = env.stage_save_fixture("minimal_v1.json");
    let loaded_fixture = store.load().unwrap().expect("load minimal_v1.json");
    assert_eq!(
        loaded_fixture.save.water_budget,
        WaterBudgetState::default()
    );

    let balance = Balance::load_default();
    let garden = Garden::from_balance(42, &balance, Timestamp::from_unix_secs(1_700_000_000));
    let mut save = GardenSave::from_garden(&garden);
    save.water_budget = WaterBudgetState {
        hour_bucket: Some(472_222),
        hour_water_gp: 650,
        hour_water_events: 4,
        day_bucket: Some(19_675),
        day_water_gp: 1_900,
        last_shell_ts: Some(Timestamp::from_unix_secs(1_700_000_300)),
    };
    store.save(&save).expect("save garden with water_budget");

    let reloaded = store.load().unwrap().expect("reload garden");
    assert_eq!(reloaded.save.water_budget, save.water_budget);
}

#[test]
fn git_poll_state_persists_counts_and_timestamps_only_and_clears_on_purge() {
    let env = TestEnv::new();
    let store = GardenStore::new(env.paths().clone());

    // Existing v1 save fixture without `git_poll` loads with default empty `GitPollState`.
    let _ = env.stage_save_fixture("minimal_v1.json");
    let loaded_fixture = store.load().unwrap().expect("load minimal_v1.json");
    assert_eq!(loaded_fixture.save.git_poll, GitPollState::default());

    let balance = Balance::load_default();
    let garden = Garden::from_balance(42, &balance, Timestamp::from_unix_secs(1_700_000_000));
    let mut save = GardenSave::from_garden(&garden);

    // When empty, `git_poll` is omitted from serialized `garden.json`.
    store.save(&save).expect("save default garden");
    let raw_empty = fs::read_to_string(env.paths().garden_file()).unwrap();
    assert!(!raw_empty.contains("git_poll"));

    save.git_poll = GitPollState {
        consent_anchor: Some(Timestamp::from_unix_secs(1_700_000_000)),
        last_poll_at: Some(Timestamp::from_unix_secs(1_700_000_300)),
        credited_commits: 7,
    };
    store.save(&save).expect("save garden with git_poll");

    let reloaded = store.load().unwrap().expect("reload garden");
    assert_eq!(reloaded.save.git_poll, save.git_poll);

    store
        .clear_git_poll_state()
        .expect("clear git_poll state on --purge");
    let after_clear = store.load().unwrap().expect("reload cleared garden");
    assert_eq!(after_clear.save.git_poll, GitPollState::default());
    let raw_after_clear = fs::read_to_string(env.paths().garden_file()).unwrap();
    assert!(!raw_after_clear.contains("git_poll"));
}
