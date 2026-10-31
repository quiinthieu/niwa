#[path = "../../../tests/support/mod.rs"]
mod support;

use proptest::prelude::*;
use std::collections::BTreeMap;
use std::fs;
use support::{workspace_root, TestEnv};
use tg_core::balance::{Balance, SignalBalance};
use tg_core::catch_up::CatchUpOptions;
use tg_core::clock::{Timestamp, TICK_SECS};
use tg_core::sim::Garden;
use tg_core::species::SpeciesCatalog;
use tg_signals::consent::{
    is_event_consented, is_signal_enabled, record_if_consented, ConsentCatalog,
};
use tg_signals::evaluator::evaluate_work_events;
use tg_signals::{
    DurationBucket, EventSpool, ShellExitClass, SignalKind, WaterBudgetState, WorkEvent,
};
use tg_store::config::AppConfig;

#[test]
fn nothing_is_collected_or_applied_before_enable() {
    let env = TestEnv::new();
    let spool = EventSpool::new(env.paths().clone());
    let mut config = AppConfig::default();
    let signal_balance = SignalBalance::default();

    let t_before = Timestamp::from_unix_secs(1_700_000_060);
    let sample_events = [
        WorkEvent::Activity {
            ts: t_before,
            active: true,
        },
        WorkEvent::GitCommit { ts: t_before },
        WorkEvent::Shell {
            ts: t_before,
            status: ShellExitClass::Ok,
            dur_bucket: DurationBucket::Medium,
        },
    ];

    // 1. Default config has every opt-in signal disabled: nothing may be collected.
    for ev in sample_events {
        assert!(!is_signal_enabled(&config, ev.signal_kind()));
        assert!(!is_event_consented(&config, ev));
        let recorded = record_if_consented(&config, &spool, ev).expect("consent gate check");
        assert!(!recorded);
    }
    assert!(
        !env.paths().events_file().exists(),
        "events.jsonl must not even be created when signals are disabled"
    );

    // 2. Hand-edited `enabled = true` without a `consented_at` timestamp is still rejected.
    config.signals.shell.enabled = true;
    config.signals.shell.consented_at = None;
    assert!(!is_signal_enabled(&config, SignalKind::Shell));
    assert!(
        !record_if_consented(&config, &spool, sample_events[2]).unwrap(),
        "missing consented_at must block collection"
    );
    assert!(!env.paths().events_file().exists());

    // 3. Enable `shell` at `t_consent = 1_700_000_120`: pre-consent events are still rejected,
    //    while events at or after `t_consent` are collected and evaluated.
    let t_consent = Timestamp::from_unix_secs(1_700_000_120);
    config.enable_signal(SignalKind::Shell, t_consent);
    assert!(is_signal_enabled(&config, SignalKind::Shell));

    let pre_consent_shell = WorkEvent::Shell {
        ts: Timestamp::from_unix_secs(1_700_000_090),
        status: ShellExitClass::Ok,
        dur_bucket: DurationBucket::Long,
    };
    let post_consent_shell = WorkEvent::Shell {
        ts: Timestamp::from_unix_secs(1_700_000_120),
        status: ShellExitClass::Ok,
        dur_bucket: DurationBucket::Medium,
    };
    assert!(!record_if_consented(&config, &spool, pre_consent_shell).unwrap());
    assert!(record_if_consented(&config, &spool, post_consent_shell).unwrap());

    let stored = spool.read().expect("read spool after consent").events;
    assert_eq!(stored, vec![post_consent_shell]);

    // 4. Even if unconsented or pre-consent events appear in an input slice, evaluator drops them.
    let mut budget = WaterBudgetState::default();
    let mixed = vec![
        sample_events[0],  // activity disabled
        sample_events[1],  // git disabled
        pre_consent_shell, // shell ts < consented_at
        post_consent_shell,
    ];
    let eval = evaluate_work_events(
        &mixed,
        &config,
        &signal_balance,
        Timestamp::from_unix_secs(1_700_000_000),
        Timestamp::from_unix_secs(1_700_000_060),
        Timestamp::from_unix_secs(1_700_000_180),
        &mut budget,
    );
    assert_eq!(eval.dropped_unconsented_events, 3);
    assert_eq!(eval.catch_up_events.len(), 1);
    assert_eq!(
        eval.total_water_gp_granted,
        signal_balance.shell_medium_water_gp
    );
    assert!(eval.retained_spool_events.is_empty());
}

#[test]
fn disable_stops_collection_and_purge_deletes_events() {
    let env = TestEnv::new();
    let spool = EventSpool::new(env.paths().clone());
    let mut config = AppConfig::default();

    let t0 = Timestamp::from_unix_secs(1_700_000_000);
    config.enable_signal(SignalKind::Shell, t0);
    config.enable_signal(SignalKind::Git, t0);

    let shell_ev = WorkEvent::Shell {
        ts: t0.add_secs(30),
        status: ShellExitClass::Ok,
        dur_bucket: DurationBucket::Fast,
    };
    let git_ev = WorkEvent::GitCommit {
        ts: t0.add_secs(45),
    };
    assert!(record_if_consented(&config, &spool, shell_ev).unwrap());
    assert!(record_if_consented(&config, &spool, git_ev).unwrap());

    // Disable `shell` without purge: new collection stops immediately, existing file remains.
    config.disable_signal(SignalKind::Shell, false);
    assert!(!record_if_consented(
        &config,
        &spool,
        WorkEvent::Shell {
            ts: t0.add_secs(60),
            status: ShellExitClass::Ok,
            dur_bucket: DurationBucket::Fast,
        }
    )
    .unwrap());
    assert_eq!(spool.read().unwrap().events, vec![shell_ev, git_ev]);

    // Purge `shell`: removes shell events while keeping git events.
    config.disable_signal(SignalKind::Shell, true);
    assert_eq!(config.signals.shell.consented_at, None);
    let purge_shell = spool.purge_signal(SignalKind::Shell).unwrap();
    assert_eq!(purge_shell.removed_events, 1);
    assert_eq!(purge_shell.remaining_events, 1);
    assert_eq!(spool.read().unwrap().events, vec![git_ev]);

    // Purge `git`: removes last event and deletes `events.jsonl`.
    config.disable_signal(SignalKind::Git, true);
    let purge_git = spool.purge_signal(SignalKind::Git).unwrap();
    assert_eq!(purge_git.removed_events, 1);
    assert_eq!(purge_git.remaining_events, 0);
    assert!(!env.paths().events_file().exists());
}

#[test]
fn true_command_loop_never_beats_honest_use_across_any_timescale() {
    let balance = Balance::load_default();
    let catalog = SpeciesCatalog::load_default();
    let t0 = Timestamp::from_unix_secs(1_700_006_400); // Exact UTC day & hour boundary.

    let mut config = AppConfig::default();
    config.enable_signal(SignalKind::Shell, t0);
    config.enable_signal(SignalKind::Git, t0);

    let simulate = |events: &[WorkEvent], end_ts: Timestamp| -> (u64, u64) {
        let mut budget = WaterBudgetState::default();
        let eval = evaluate_work_events(
            events,
            &config,
            &balance.signals,
            t0,
            t0,
            end_ts,
            &mut budget,
        );
        let mut garden = Garden::from_balance(0xBEEF, &balance, t0);
        garden.plant_seed(0, "daisy").expect("plant daisy");
        garden.catch_up_with_events(
            end_ts,
            &eval.catch_up_events,
            CatchUpOptions::default(),
            &balance,
            &catalog,
        );
        let plant_gp = garden.plots()[0].plant().unwrap().total_gp;
        (eval.total_water_gp_granted, plant_gp)
    };

    // 1. One-minute window: 600 rapid `true` commands vs 1 honest `medium` test + 1 `git_commit`.
    let one_min_end = t0.add_secs(60);
    let spam_1m: Vec<WorkEvent> = (1..=600)
        .map(|i| WorkEvent::Shell {
            ts: t0.add_secs((i / 10).max(1)),
            status: ShellExitClass::Ok,
            dur_bucket: DurationBucket::Instant,
        })
        .collect();
    let honest_1m = vec![
        WorkEvent::Shell {
            ts: t0.add_secs(30),
            status: ShellExitClass::Ok,
            dur_bucket: DurationBucket::Medium,
        },
        WorkEvent::GitCommit {
            ts: t0.add_secs(50),
        },
    ];
    let (spam_1m_water, spam_1m_gp) = simulate(&spam_1m, one_min_end);
    let (honest_1m_water, honest_1m_gp) = simulate(&honest_1m, one_min_end);
    assert!(
        spam_1m_water < honest_1m_water,
        "1m true-loop water ({spam_1m_water}) must lose to honest use ({honest_1m_water})"
    );
    assert!(
        spam_1m_gp < honest_1m_gp,
        "1m true-loop total GP ({spam_1m_gp}) must lose to honest use ({honest_1m_gp})"
    );

    // 2. One-hour window: optimal 5s-spaced `true` loop (720 commands) vs 1 hour of honest coding.
    let one_hr_end = t0.add_secs(3_600);
    let spam_1h: Vec<WorkEvent> = (1..=720)
        .map(|i| WorkEvent::Shell {
            ts: t0.add_secs(i * 5),
            status: ShellExitClass::Ok,
            dur_bucket: DurationBucket::Instant,
        })
        .collect();
    let mut honest_1h = Vec::new();
    for minute in 1..=60 {
        let dur_bucket = if minute % 15 == 0 {
            DurationBucket::Long
        } else if minute % 5 == 0 {
            DurationBucket::Medium
        } else {
            DurationBucket::Fast
        };
        honest_1h.push(WorkEvent::Shell {
            ts: t0.add_secs(minute * 60),
            status: ShellExitClass::Ok,
            dur_bucket,
        });
        if minute % 20 == 0 {
            honest_1h.push(WorkEvent::GitCommit {
                ts: t0.add_secs(minute * 60),
            });
        }
    }
    let (spam_1h_water, spam_1h_gp) = simulate(&spam_1h, one_hr_end);
    let (honest_1h_water, honest_1h_gp) = simulate(&honest_1h, one_hr_end);
    assert!(
        spam_1h_water < honest_1h_water,
        "1h true-loop water ({spam_1h_water}) must lose to honest hour ({honest_1h_water})"
    );
    assert!(
        spam_1h_gp < honest_1h_gp,
        "1h true-loop total GP ({spam_1h_gp}) must lose to honest hour ({honest_1h_gp})"
    );

    // 3. Full 24-hour day: nonstop 24/7 optimal 5s-spaced `true` loop (17,280 commands!)
    //    vs a normal 6-hour honest workday (leaving 18 hours completely idle).
    let day_end = t0.add_secs(86_400);
    let spam_24h: Vec<WorkEvent> = (1..=17_280)
        .map(|i| WorkEvent::Shell {
            ts: t0.add_secs(i * 5),
            status: ShellExitClass::Ok,
            dur_bucket: DurationBucket::Instant,
        })
        .collect();
    let mut honest_workday = Vec::new();
    for hour in 0..6 {
        let hour_base = t0.add_secs(hour * 3_600);
        for minute in 1..=60 {
            let dur_bucket = if minute % 12 == 0 {
                DurationBucket::Long
            } else if minute % 4 == 0 {
                DurationBucket::Medium
            } else {
                DurationBucket::Fast
            };
            honest_workday.push(WorkEvent::Shell {
                ts: hour_base.add_secs(minute * 60),
                status: ShellExitClass::Ok,
                dur_bucket,
            });
            if minute % 20 == 0 {
                honest_workday.push(WorkEvent::GitCommit {
                    ts: hour_base.add_secs(minute * 60),
                });
            }
        }
    }
    let (spam_24h_water, _) = simulate(&spam_24h, day_end);
    let (honest_day_water, _) = simulate(&honest_workday, day_end);
    assert!(
        spam_24h_water < honest_day_water,
        "even 24h nonstop true-loop water ({spam_24h_water}) must lose to a 6h honest workday ({honest_day_water})"
    );
    assert_eq!(honest_day_water, balance.signals.daily_water_cap_gp);
}

#[test]
fn out_of_window_events_feed_diagnostics_without_consuming_water_budget() {
    let balance = Balance::load_default();
    let catalog = SpeciesCatalog::load_default();
    let t_consent = Timestamp::from_unix_secs(1_700_000_000);
    let created_at = Timestamp::from_unix_secs(1_700_001_000);
    let now = created_at.add_secs(600);

    let mut config = AppConfig::default();
    config.enable_signal(SignalKind::Git, t_consent);

    let events = vec![
        WorkEvent::GitCommit {
            ts: Timestamp::from_unix_secs(1_700_000_500), // consented, but < created_at
        },
        WorkEvent::GitCommit {
            ts: now.add_secs(120), // future: > now
        },
    ];

    let mut budget = WaterBudgetState::default();
    let eval = evaluate_work_events(
        &events,
        &config,
        &balance.signals,
        created_at,
        created_at,
        now,
        &mut budget,
    );

    assert_eq!(eval.total_water_gp_granted, 0);
    assert_eq!(
        budget,
        WaterBudgetState::default(),
        "dropped pre-creation and future events must not consume hourly/daily water caps"
    );
    assert_eq!(eval.catch_up_events.len(), 2);

    let mut garden = Garden::from_balance(7, &balance, created_at);
    let outcome = garden.catch_up_with_events(
        now,
        &eval.catch_up_events,
        CatchUpOptions::default(),
        &balance,
        &catalog,
    );
    assert_eq!(outcome.diagnostics.dropped_pre_creation_events, 1);
    assert_eq!(outcome.diagnostics.dropped_future_events, 1);
}

#[test]
fn privacy_docs_and_shared_consent_catalog_cover_every_signal_and_field() {
    let consent = ConsentCatalog::load_default();
    assert!(!consent.baseline().summary.is_empty());
    assert!(!consent.baseline().stores.is_empty());
    assert!(!consent.baseline().never_stores.is_empty());

    let privacy_doc =
        fs::read_to_string(workspace_root().join("docs").join("privacy.md")).expect("privacy.md");

    for required_token in [
        "`time`",
        "`activity`",
        "`git`",
        "`shell`",
        "`git_commit`",
        "`ts`",
        "`active`",
        "`status`",
        "`dur_bucket`",
        "\"instant\"",
        "\"fast\"",
        "\"medium\"",
        "\"long\"",
        "`water_budget`",
        "`dropped_future_events`",
        "`dropped_pre_creation_events`",
        "`corrupt_spool_lines`",
        "`clock_skew_events`",
        "niwa signals disable",
        "--purge",
        "niwa reset --all",
    ] {
        assert!(
            privacy_doc.contains(required_token),
            "docs/privacy.md is missing required specification token: {required_token}"
        );
    }

    for kind in SignalKind::ALL {
        let disclosure = consent.signal(kind);
        assert!(!disclosure.display_name.is_empty());
        assert!(!disclosure.summary.is_empty());
        assert!(!disclosure.stores.is_empty());
        assert!(!disclosure.never_stores.is_empty());
        assert!(!disclosure.prompt.is_empty());

        let rendered = disclosure.render_disclosure_text(kind);
        assert!(rendered.contains(kind.as_str()));
        assert!(rendered.contains(&disclosure.summary));
        assert!(rendered.contains(&disclosure.prompt));
    }
}

fn arb_duration_bucket() -> impl Strategy<Value = DurationBucket> {
    prop_oneof![
        Just(DurationBucket::Instant),
        Just(DurationBucket::Fast),
        Just(DurationBucket::Medium),
        Just(DurationBucket::Long),
    ]
}

fn arb_work_event(base_secs: i64, max_offset_secs: i64) -> impl Strategy<Value = WorkEvent> {
    (
        1_i64..=max_offset_secs,
        0_u8..3,
        any::<bool>(),
        arb_duration_bucket(),
    )
        .prop_map(move |(offset, variant, flag, dur_bucket)| {
            let ts = Timestamp::from_unix_secs(base_secs + offset);
            match variant {
                0 => WorkEvent::Activity { ts, active: flag },
                1 => WorkEvent::GitCommit { ts },
                _ => WorkEvent::Shell {
                    ts,
                    status: if flag {
                        ShellExitClass::Ok
                    } else {
                        ShellExitClass::Err
                    },
                    dur_bucket,
                },
            }
        })
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 192,
        failure_persistence: None,
        .. ProptestConfig::default()
    })]

    #[test]
    fn spam_never_exceeds_hourly_or_daily_water_caps(
        hourly_cap in 50_u64..=2_500,
        extra_daily_cap in 0_u64..=5_000,
        half_life in 1_u64..=25,
        min_spacing in 0_u64..=15,
        events in prop::collection::vec(arb_work_event(1_700_006_400, 3 * 86_400), 1..=350),
    ) {
        let base_ts = Timestamp::from_unix_secs(1_700_006_400);
        let end_ts = base_ts.add_secs(3 * 86_400);
        let daily_cap = hourly_cap + extra_daily_cap;

        let signal_balance = SignalBalance {
            commit_water_gp: 600,
            shell_instant_water_gp: 25,
            shell_fast_water_gp: 80,
            shell_medium_water_gp: 180,
            shell_long_water_gp: 350,
            shell_min_spacing_secs: min_spacing,
            diminishing_half_life_events: half_life,
            hourly_water_cap_gp: hourly_cap,
            daily_water_cap_gp: daily_cap,
        };

        let mut config = AppConfig::default();
        for kind in SignalKind::ALL {
            config.enable_signal(kind, base_ts);
        }

        let mut budget = WaterBudgetState::default();
        let eval = evaluate_work_events(
            &events,
            &config,
            &signal_balance,
            base_ts,
            base_ts,
            end_ts,
            &mut budget,
        );

        let mut water_by_hour: BTreeMap<i64, u64> = BTreeMap::new();
        let mut water_by_day: BTreeMap<i64, u64> = BTreeMap::new();
        for ev in &eval.catch_up_events {
            let hour = ev.timestamp.as_unix_secs().div_euclid(3_600);
            let day = ev.timestamp.as_unix_secs().div_euclid(86_400);
            *water_by_hour.entry(hour).or_insert(0) += ev.water_gp;
            *water_by_day.entry(day).or_insert(0) += ev.water_gp;
        }

        for (&hour, &water) in &water_by_hour {
            prop_assert!(
                water <= hourly_cap,
                "UTC hour {hour} awarded {water} water GP, exceeding hourly cap {hourly_cap}"
            );
        }
        for (&day, &water) in &water_by_day {
            prop_assert!(
                water <= daily_cap,
                "UTC day {day} awarded {water} water GP, exceeding daily cap {daily_cap}"
            );
        }
    }

    #[test]
    fn evaluator_and_catch_up_batching_invariance_holds_across_compactions(
        seed in any::<u64>(),
        events in prop::collection::vec(arb_work_event(1_700_006_400, 14_400), 0..=120),
        mut checkpoints in prop::collection::vec(1_i64..=14_400, 1..=16),
    ) {
        let base_ts = Timestamp::from_unix_secs(1_700_006_400);
        let final_ts = base_ts.add_secs(14_400); // 4 hours = 240 ticks (exact tick boundary)
        checkpoints.push(14_400);
        checkpoints.sort_unstable();
        checkpoints.dedup();

        let balance = Balance::load_default();
        let catalog = SpeciesCatalog::load_default();
        let mut config = AppConfig::default();
        for kind in SignalKind::ALL {
            config.enable_signal(kind, base_ts);
        }

        // 1. One-shot evaluation and catch-up at `final_ts`.
        let mut one_shot_garden = Garden::from_balance(seed, &balance, base_ts);
        one_shot_garden.plant_seed(0, "daisy").unwrap();
        let mut one_shot_budget = WaterBudgetState::default();
        let one_shot_eval = evaluate_work_events(
            &events,
            &config,
            &balance.signals,
            one_shot_garden.created_at(),
            one_shot_garden.last_tick_at(),
            final_ts,
            &mut one_shot_budget,
        );
        one_shot_garden.catch_up_with_events(
            final_ts,
            &one_shot_eval.catch_up_events,
            CatchUpOptions::default(),
            &balance,
            &catalog,
        );

        // 2. Incremental runs across arbitrary sub-minute / multi-minute / multi-hour checkpoints,
        //    simulating real spool arrivals + compactions + persisted `WaterBudgetState`.
        let mut inc_garden = Garden::from_balance(seed, &balance, base_ts);
        inc_garden.plant_seed(0, "daisy").unwrap();
        let mut inc_budget = WaterBudgetState::default();
        let mut spool_buffer: Vec<WorkEvent> = Vec::new();
        let mut prev_checkpoint_secs = 0_i64;

        for cp_offset in checkpoints {
            let cp_now = base_ts.add_secs(cp_offset);
            for &ev in &events {
                let offset = ev.timestamp().as_unix_secs() - base_ts.as_unix_secs();
                if offset > prev_checkpoint_secs && offset <= cp_offset {
                    spool_buffer.push(ev);
                }
            }
            prev_checkpoint_secs = cp_offset;

            let step_eval = evaluate_work_events(
                &spool_buffer,
                &config,
                &balance.signals,
                inc_garden.created_at(),
                inc_garden.last_tick_at(),
                cp_now,
                &mut inc_budget,
            );
            inc_garden.catch_up_with_events(
                cp_now,
                &step_eval.catch_up_events,
                CatchUpOptions::default(),
                &balance,
                &catalog,
            );
            spool_buffer = step_eval.retained_spool_events;

            // Verify partial-tick retention invariant at intermediate non-minute checkpoints.
            let rem_secs = (cp_now.as_unix_secs() - base_ts.as_unix_secs()) % TICK_SECS;
            if rem_secs == 0 {
                prop_assert!(spool_buffer.is_empty());
            }
        }

        prop_assert_eq!(inc_budget, one_shot_budget);
        prop_assert_eq!(inc_garden, one_shot_garden);
    }
}
