use proptest::prelude::*;
use std::time::{Duration, Instant};
use tg_core::balance::{Balance, MULTIPLIER_BP_SCALE};
use tg_core::catch_up::{CatchUpEvent, CatchUpOptions, ClockSkewNote};
use tg_core::clock::{Clock, FakeClock, Timestamp, TICKS_PER_DAY, TICKS_PER_HOUR, TICK_SECS};
use tg_core::model::Stage;
use tg_core::season::{Hemisphere, Season, SeasonMode};
use tg_core::sim::Garden;
use tg_core::species::SpeciesCatalog;

fn seeded_nine_plot_garden(seed: u64, start: Timestamp, balance: &Balance) -> Garden {
    let mut garden = Garden::from_balance(seed, balance, start);
    let starter_species = [
        "daisy",
        "lavender",
        "camellia",
        "moss",
        "clover",
        "marigold",
        "red_maple",
        "moonflower",
        "chamomile",
    ];
    for (idx, species_id) in starter_species.iter().enumerate() {
        garden.plant_seed(idx, *species_id).unwrap();
    }
    garden
}

#[test]
fn thirty_day_catch_up_matches_daily_and_hourly_batches() {
    let balance = Balance::load_default();
    let catalog = SpeciesCatalog::load_embedded().expect("embedded species catalog");
    // Start in late May (`2026-05-20T00:00:00Z` = 1_779_235_200) so 30 days crosses May -> June
    // (Spring -> Summer in Northern Hemisphere), exercising calendar season transitions too!
    let start = Timestamp::from_unix_secs(1_779_235_200);
    let options = CatchUpOptions {
        season_mode: SeasonMode::Calendar,
        hemisphere: Hemisphere::North,
        dormant_after_days_override: None,
    };

    let base_garden = seeded_nine_plot_garden(0xABCD_EF01_2345_6789, start, &balance);

    // 1. One 30-day batch (`43_200` ticks).
    let mut one_batch = base_garden.clone();
    let end = start.add_ticks(30 * TICKS_PER_DAY);
    let outcome_one = one_batch.catch_up(end, options, &balance, &catalog);
    assert_eq!(outcome_one.ticks_simulated, 30 * TICKS_PER_DAY);
    assert_eq!(outcome_one.chunks_processed, 30);
    assert_eq!(
        outcome_one.full_offline_ticks,
        balance.catch_up.offline_cap_ticks()
    );
    assert_eq!(
        outcome_one.tail_offline_ticks,
        balance.dormant_after_ticks() - balance.catch_up.offline_cap_ticks()
    );
    assert_eq!(
        outcome_one.dormant_ticks,
        30 * TICKS_PER_DAY - balance.dormant_after_ticks()
    );
    assert!(one_batch.is_dormant());

    // 2. Thirty 1-day batches (`1_440` ticks x 30).
    let mut daily_batches = base_garden.clone();
    let clock_daily = FakeClock::new(start);
    for _ in 0..30 {
        let now = clock_daily.advance_ticks(TICKS_PER_DAY);
        daily_batches.catch_up(now, options, &balance, &catalog);
    }

    // 3. Seven-hundred-twenty 1-hour batches (`60` ticks x 720).
    let mut hourly_batches = base_garden;
    let clock_hourly = FakeClock::new(start);
    for _ in 0..(30 * 24) {
        let now = clock_hourly.advance_ticks(TICKS_PER_HOUR);
        hourly_batches.catch_up(now, options, &balance, &catalog);
    }

    assert_eq!(one_batch, daily_batches);
    assert_eq!(one_batch, hourly_batches);
}

#[test]
fn offline_cap_and_tail_factor_scale_growth_precisely_before_dormancy() {
    let mut balance = Balance::load_default();
    // Neutralize season multiplier so GP math is crystal-clear (`100` full, `50` tail).
    balance.seasons.preferred_season_multiplier_bp = MULTIPLIER_BP_SCALE;
    balance.seasons.off_season_multiplier_bp = MULTIPLIER_BP_SCALE;
    let catalog = SpeciesCatalog::load_embedded().expect("embedded catalog");

    let start = Timestamp::from_unix_secs(1_775_001_600);
    let clock = FakeClock::new(start);
    let mut garden = Garden::from_balance(42, &balance, clock.now());
    // `red_maple` is Rare with large thresholds and `growth_multiplier = 1.0`? Let's use an
    // uncatalogued id so it uses `balance.default_profile` (`1.0x`, 720_000 GP to bloom).
    garden.plant_seed(0, "neutral_test_herb").unwrap();

    // First 72 hours (4_320 ticks): full base = 4_320 * 100 = 432_000 GP.
    clock.advance_ticks(72 * TICKS_PER_HOUR);
    let out_cap = garden.catch_up(clock.now(), CatchUpOptions::default(), &balance, &catalog);
    assert_eq!(out_cap.full_offline_ticks, 72 * TICKS_PER_HOUR);
    assert_eq!(out_cap.tail_offline_ticks, 0);
    assert_eq!(out_cap.dormant_ticks, 0);
    assert!(!garden.is_dormant());
    assert_eq!(garden.plots()[0].plant().unwrap().total_gp, 432_000);

    // Next 96 hours (hours 72..168 = days 4..=7 = 5_760 ticks): tail factor 0.5 => 5_760 * 50 = 288_000 GP.
    // Total = 432_000 + 288_000 = 720_000 GP (blooms!) and enters dormancy at 7 days.
    clock.advance_ticks(96 * TICKS_PER_HOUR);
    let out_tail = garden.catch_up(clock.now(), CatchUpOptions::default(), &balance, &catalog);
    assert_eq!(out_tail.full_offline_ticks, 0);
    assert_eq!(out_tail.tail_offline_ticks, 96 * TICKS_PER_HOUR);
    assert_eq!(out_tail.dormant_ticks, 0);
    assert!(garden.is_dormant());
    assert_eq!(garden.plots()[0].plant().unwrap().total_gp, 720_000);
    assert_eq!(garden.plots()[0].plant().unwrap().stage, Stage::Blooming);

    // Next 48 hours while dormant: 0 GP gained, nothing lost.
    clock.advance_ticks(48 * TICKS_PER_HOUR);
    let out_dormant = garden.catch_up(clock.now(), CatchUpOptions::default(), &balance, &catalog);
    assert_eq!(out_dormant.dormant_ticks, 48 * TICKS_PER_HOUR);
    assert_eq!(garden.plots()[0].plant().unwrap().total_gp, 720_000);

    // An active event wakes the garden and resets `inactive_ticks` back to full offline trickle!
    let event_time = clock.advance_ticks(1);
    let wake_events = [CatchUpEvent::active_at(event_time)];
    let out_wake = garden.catch_up_with_events(
        clock.now(),
        &wake_events,
        CatchUpOptions::default(),
        &balance,
        &catalog,
    );
    assert_eq!(out_wake.active_ticks, 1);
    assert!(!garden.is_dormant());
    assert_eq!(garden.inactive_ticks(), 0);
    assert_eq!(garden.plots()[0].plant().unwrap().total_gp, 720_250);
}

#[test]
fn clock_skew_backwards_is_ignored_and_noted_without_mutating_garden() {
    let balance = Balance::load_default();
    let catalog = SpeciesCatalog::load_embedded().expect("embedded catalog");
    let clock = FakeClock::from_unix_secs(1_780_000_000);
    let mut garden = Garden::from_balance(7, &balance, clock.now());
    garden.plant_seed(0, "daisy").unwrap();

    clock.advance_ticks(120);
    garden.catch_up(clock.now(), CatchUpOptions::default(), &balance, &catalog);
    let snapshot_before_skew = garden.clone();
    let latest_ts = garden.last_tick_at();

    // Move clock backwards by 5 minutes (300 seconds).
    let skewed_now = clock.advance_secs(-300);
    let outcome = garden.catch_up(skewed_now, CatchUpOptions::default(), &balance, &catalog);

    assert_eq!(garden, snapshot_before_skew);
    assert_eq!(outcome.ticks_simulated, 0);
    assert_eq!(outcome.diagnostics.clock_skew_events, 1);
    assert_eq!(
        outcome.clock_skew,
        Some(ClockSkewNote {
            observed_now: skewed_now,
            last_tick_at: latest_ts,
            backwards_secs: 300,
        })
    );
    assert!(outcome.summary.has_activity_to_report());
    assert!(outcome.summary.render_text().contains("clock skew"));
}

#[test]
fn sub_tick_elapsed_seconds_preserve_remainder_and_filter_out_of_range_events() {
    let balance = Balance::load_default();
    let catalog = SpeciesCatalog::load_embedded().expect("embedded catalog");
    let created = Timestamp::from_unix_secs(1_780_000_000);
    let clock = FakeClock::new(created);
    let mut garden = Garden::from_balance(123, &balance, clock.now());
    garden.plant_seed(0, "moss").unwrap();

    // 35 seconds (< 1 tick): 0 ticks simulated, last_tick_at stays at `created`.
    clock.advance_secs(35);
    let sub1 = garden.catch_up(clock.now(), CatchUpOptions::default(), &balance, &catalog);
    assert_eq!(sub1.ticks_simulated, 0);
    assert_eq!(garden.last_tick_at(), created);
    assert!(!sub1.summary.has_activity_to_report());

    // Another 35 seconds (total 70s = 1 tick + 10s remainder).
    let now = clock.advance_secs(35);
    let events = [
        // Pre-creation event -> dropped and counted.
        CatchUpEvent::with_water(created.add_secs(-10), true, 50),
        // Valid in-window event on tick 0 (`created + 45s` falls in `(created ..= created + 60]`).
        CatchUpEvent::with_water(created.add_secs(45), true, 50),
        // Future event (`> now`) -> dropped and counted.
        CatchUpEvent::with_water(now.add_secs(120), true, 50),
    ];
    let sub2 =
        garden.catch_up_with_events(now, &events, CatchUpOptions::default(), &balance, &catalog);
    assert_eq!(sub2.ticks_simulated, 1);
    assert_eq!(sub2.active_ticks, 1);
    assert_eq!(sub2.diagnostics.dropped_pre_creation_events, 1);
    assert_eq!(sub2.diagnostics.dropped_future_events, 1);
    assert_eq!(garden.last_tick_at(), created.add_secs(TICK_SECS));

    // Another 50 seconds completes the second tick (10s remainder + 50s = 60s).
    clock.advance_secs(50);
    let sub3 = garden.catch_up(clock.now(), CatchUpOptions::default(), &balance, &catalog);
    assert_eq!(sub3.ticks_simulated, 1);
    assert_eq!(garden.tick_index(), 2);
}

#[test]
fn thirty_day_catch_up_bench_stays_under_section_7_target() {
    let balance = Balance::load_default();
    let catalog = SpeciesCatalog::load_embedded().expect("embedded catalog");
    let start = Timestamp::from_unix_secs(1_779_235_200);
    let end = start.add_ticks(30 * TICKS_PER_DAY);
    let options = CatchUpOptions {
        season_mode: SeasonMode::Calendar,
        hemisphere: Hemisphere::North,
        dormant_after_days_override: None,
    };

    let template = seeded_nine_plot_garden(0xFEED_FACE_CAFE_BEEF, start, &balance);

    // Warm-up run.
    let mut warmup = template.clone();
    warmup.catch_up(end, options, &balance, &catalog);

    // Measure 5 independent 30-day catch-ups (`43_200` ticks across 9 planted plots).
    // PLAN §7 target: under 200 ms in release on the CI runner.
    let target = Duration::from_millis(200);
    let mut worst = Duration::ZERO;
    for _ in 0..5 {
        let mut garden = template.clone();
        let t0 = Instant::now();
        let out = garden.catch_up(end, options, &balance, &catalog);
        let elapsed = t0.elapsed();
        assert_eq!(out.ticks_simulated, 30 * TICKS_PER_DAY);
        if elapsed > worst {
            worst = elapsed;
        }
    }

    assert!(
        worst < target,
        "30-day catch-up took {worst:?}, exceeding PLAN §7 200ms target"
    );
}

#[test]
fn while_you_were_away_summary_is_golden() {
    let balance = Balance::load_default();
    let catalog = SpeciesCatalog::load_embedded().expect("embedded catalog");
    // `2026-05-28T00:00:00Z` = 1_779_926_400 (Spring in North; +4 days crosses June 1 into Summer).
    let start = Timestamp::from_unix_secs(1_779_926_400);
    let options = CatchUpOptions {
        season_mode: SeasonMode::Calendar,
        hemisphere: Hemisphere::North,
        dormant_after_days_override: None,
    };

    let mut base = Garden::from_balance(2026, &balance, start);
    base.plant_seed(0, "daisy").unwrap();
    base.plant_seed(1, "lavender").unwrap();
    base.plant_seed(2, "red_maple").unwrap();

    // Scenario 1: Sub-minute return (< 60s).
    let mut g_sub = base.clone();
    let s_sub = g_sub
        .catch_up(start.add_secs(20), options, &balance, &catalog)
        .summary
        .render_text();

    // Scenario 2: 18 hours away (within 72h full offline cap).
    let mut g_18h = base.clone();
    let s_18h = g_18h
        .catch_up(
            start.add_ticks(18 * TICKS_PER_HOUR),
            options,
            &balance,
            &catalog,
        )
        .summary
        .render_text();

    // Scenario 3: 5d 1h away (crosses 72h offline_cap into tail pace AND May -> June season shift, blooming Daisy before 7d dormancy).
    let mut g_5d = base.clone();
    let s_5d = g_5d
        .catch_up(
            start.add_ticks(5 * TICKS_PER_DAY + TICKS_PER_HOUR),
            options,
            &balance,
            &catalog,
        )
        .summary
        .render_text();
    assert_eq!(g_5d.plots()[0].plant().unwrap().stage, Stage::Blooming);
    assert!(!g_5d.is_dormant());

    // Scenario 4: 30 days away (crosses 72h cap, 7d dormancy, and Spring -> Summer).
    let mut g_30d = base.clone();
    let out_30d = g_30d.catch_up(
        start.add_ticks(30 * TICKS_PER_DAY),
        options,
        &balance,
        &catalog,
    );
    assert_eq!(out_30d.summary.season_before, Season::Spring);
    assert_eq!(out_30d.summary.season_after, Season::Summer);
    let s_30d = out_30d.summary.render_text();

    // Scenario 5: Backwards clock skew (-5m).
    let mut g_skew = base;
    let s_skew = g_skew
        .catch_up(start.add_secs(-300), options, &balance, &catalog)
        .summary
        .render_text();

    let combined = [
        format!("=== sub_minute ===\n{s_sub}"),
        format!("=== away_18h ===\n{s_18h}"),
        format!("=== away_5d_tail ===\n{s_5d}"),
        format!("=== away_30d_dormant ===\n{s_30d}"),
        format!("=== clock_skew ===\n{s_skew}"),
    ]
    .join("\n\n");

    insta::assert_snapshot!("while_you_were_away_summaries_golden", combined);
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 128,
        failure_persistence: None,
        .. ProptestConfig::default()
    })]

    #[test]
    fn catch_up_batching_invariance_holds_across_random_splits_and_events(
        seed in any::<u64>(),
        offline_cap_hours in 0_u32..=6,
        tail_bp in 0_u32..=10_000_u32,
        dormant_days in 1_u32..=2,
        chunk_ticks in 1_u64..=120,
        total_ticks in 1_u64..=3_200,
        raw_splits in prop::collection::vec(1_u64..=3_200, 1..=12),
        raw_events in prop::collection::vec((1_u64..=3_200, any::<bool>(), 0_u64..=120), 0..=20),
        hemisphere_south in any::<bool>(),
        garden_mode in any::<bool>(),
    ) {
        let mut balance = Balance::load_default();
        balance.catch_up.offline_cap_hours = offline_cap_hours;
        balance.catch_up.offline_tail_factor_bp = tail_bp;
        balance.catch_up.chunk_ticks = chunk_ticks;
        balance.dormant_after_days = dormant_days;

        let catalog = SpeciesCatalog::load_embedded().unwrap();
        let start = Timestamp::from_unix_secs(1_779_926_400);
        let options = CatchUpOptions {
            season_mode: if garden_mode { SeasonMode::Garden } else { SeasonMode::Calendar },
            hemisphere: if hemisphere_south { Hemisphere::South } else { Hemisphere::North },
            dormant_after_days_override: None,
        };

        let events: Vec<CatchUpEvent> = raw_events
            .into_iter()
            .filter(|(tick_offset, _, _)| *tick_offset <= total_ticks)
            .map(|(tick_offset, active, water_gp)| {
                // Place event inside tick `tick_offset - 1` window `(start + (k)*60 ..= start + (k+1)*60]`.
                let ts = start.add_secs((tick_offset as i64) * TICK_SECS - 15);
                CatchUpEvent::with_water(ts, active, water_gp)
            })
            .collect();

        let mut base = Garden::new(seed, 3, start);
        base.plant_seed(0, "daisy").unwrap();
        base.plant_seed(1, "lavender").unwrap();
        base.plant_seed(2, "camellia").unwrap();

        let mut one_shot = base.clone();
        let end = start.add_ticks(total_ticks);
        one_shot.catch_up_with_events(end, &events, options, &balance, &catalog);

        let mut split_ticks: Vec<u64> = raw_splits
            .into_iter()
            .map(|t| t.min(total_ticks))
            .collect();
        split_ticks.push(total_ticks);
        split_ticks.sort_unstable();
        split_ticks.dedup();

        let mut batched = base;
        let mut prev_boundary = start;
        for boundary_tick in split_ticks {
            let boundary_ts = start.add_ticks(boundary_tick);
            let slice_events: Vec<CatchUpEvent> = events
                .iter()
                .copied()
                .filter(|ev| ev.timestamp > prev_boundary && ev.timestamp <= boundary_ts)
                .collect();
            batched.catch_up_with_events(boundary_ts, &slice_events, options, &balance, &catalog);
            prev_boundary = boundary_ts;
        }

        prop_assert_eq!(one_shot, batched);
    }
}
