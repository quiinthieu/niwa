use rand::RngCore;
use std::collections::BTreeMap;
use tg_core::balance::{
    multiplier_to_bp, Balance, BalanceError, DropBalance, GrowthProfile, SeasonBalance,
    MULTIPLIER_BP_SCALE,
};
use tg_core::clock::{Clock, FakeClock, Timestamp, TICKS_PER_DAY, TICK_SECS};
use tg_core::model::{SimError, Stage};
use tg_core::rng::{rng_for_stream, rng_for_tick};
use tg_core::sim::{Garden, StageTransition, TickInput};

fn small_balance(thresholds: [u64; 4], dormant_after_days: u32) -> Balance {
    Balance {
        default_plots: 4,
        base_gp_per_tick: 10,
        sun_gp_per_tick: 15,
        default_profile: GrowthProfile::new(thresholds, MULTIPLIER_BP_SCALE)
            .expect("valid test growth profile"),
        default_season_multiplier_bp: MULTIPLIER_BP_SCALE,
        dormant_after_days,
        seasons: SeasonBalance::default(),
        drops: DropBalance::default(),
    }
}

#[test]
fn default_balance_toml_loads_and_matches_work_week_target() {
    let balance = Balance::load_default();
    assert!(balance.default_plots >= 1);
    assert!(balance.base_gp_per_tick >= 1);
    assert_eq!(balance.dormant_after_days, 7);
    assert_eq!(balance.dormant_after_ticks(), 7 * TICKS_PER_DAY);

    // Default time-only bloom should sit within a work week (~5 days = 7_200 ticks),
    // strictly before the 7-day dormancy window (10_080 ticks).
    let ticks_to_bloom = balance
        .default_profile
        .total_to_bloom()
        .div_ceil(balance.base_gp_per_tick);
    assert_eq!(ticks_to_bloom, 5 * TICKS_PER_DAY);
    assert!(ticks_to_bloom < balance.dormant_after_ticks());
}

#[test]
fn balance_validation_rejects_invalid_toml_and_bounds() {
    assert!(matches!(
        Balance::from_toml_str("not valid = [toml"),
        Err(BalanceError::ParseToml(_))
    ));

    let zero_plots = r#"
        [garden]
        default_plots = 0
        [growth]
        base_gp_per_tick = 10
        sun_gp_per_tick = 10
        default_stage_thresholds = [10, 20, 30, 40]
        default_species_multiplier = 1.0
        default_season_multiplier = 1.0
        [dormancy]
        dormant_after_days = 7
    "#;
    assert_eq!(
        Balance::from_toml_str(zero_plots),
        Err(BalanceError::ZeroDefaultPlots)
    );

    let zero_threshold = r#"
        [garden]
        default_plots = 4
        [growth]
        base_gp_per_tick = 10
        sun_gp_per_tick = 10
        default_stage_thresholds = [10, 0, 30, 40]
        default_species_multiplier = 1.0
        default_season_multiplier = 1.0
        [dormancy]
        dormant_after_days = 7
    "#;
    assert_eq!(
        Balance::from_toml_str(zero_threshold),
        Err(BalanceError::InvalidStageThreshold {
            stage_index: 1,
            value: 0,
        })
    );

    assert_eq!(
        multiplier_to_bp("species", -0.5),
        Err(BalanceError::InvalidMultiplier { field: "species" })
    );
    assert_eq!(
        multiplier_to_bp("season", f64::NAN),
        Err(BalanceError::InvalidMultiplier { field: "season" })
    );
}

#[test]
fn plant_progresses_through_every_stage_transition_in_order() {
    // Thresholds:
    // Seed -> Sprout:     25 GP (ticks 1, 2 -> 20 GP; tick 3 -> 30 GP => Sprout with 5 GP carry-over)
    // Sprout -> Young:    30 GP (5 + 10 + 10 = 25 on tick 5; tick 6 -> 35 => Young with 5 GP carry-over)
    // Young -> Mature:    35 GP (5 + 10 + 10 + 10 = 35 on tick 9 => Mature with 0 GP carry-over)
    // Mature -> Blooming: 20 GP (0 + 10 = 10 on tick 10; tick 11 -> 20 => Blooming with 0 GP carry-over)
    let balance = small_balance([25, 30, 35, 20], 7);
    let clock = FakeClock::from_unix_secs(1_700_000_000);
    let mut garden = Garden::from_clock(0xCAFE_BABE, &balance, &clock);

    garden.plant_seed(0, "moss").expect("plant moss in plot 0");
    let initial = garden.plots()[0].plant().expect("planted");
    assert_eq!(initial.stage, Stage::Seed);
    assert_eq!(initial.stage_gp, 0);
    assert_eq!(initial.total_gp, 0);
    assert_eq!(initial.bloomed_at, None);
    assert_eq!(initial.bloom_seed, None);

    // Ticks 1..=2: stays Seed (20 / 25 GP).
    for _ in 0..2 {
        clock.advance_ticks(1);
        let outcome = garden.tick(TickInput::idle(), &balance);
        assert!(outcome.transitions.is_empty());
    }
    let p = garden.plots()[0].plant().unwrap();
    assert_eq!(p.stage, Stage::Seed);
    assert_eq!(p.stage_gp, 20);
    assert_eq!(p.total_gp, 20);

    // Tick 3 (tick_index 2): Seed -> Sprout, carry-over = 5 GP.
    clock.advance_ticks(1);
    let out = garden.tick(TickInput::idle(), &balance);
    assert_eq!(
        out.transitions,
        vec![StageTransition {
            plot_index: 0,
            from: Stage::Seed,
            to: Stage::Sprout,
        }]
    );
    let p = garden.plots()[0].plant().unwrap();
    assert_eq!(p.stage, Stage::Sprout);
    assert_eq!(p.stage_gp, 5);
    assert_eq!(p.total_gp, 30);

    // Ticks 4..=5: stays Sprout (25 / 30 GP).
    for _ in 0..2 {
        clock.advance_ticks(1);
        assert!(garden
            .tick(TickInput::idle(), &balance)
            .transitions
            .is_empty());
    }
    assert_eq!(garden.plots()[0].plant().unwrap().stage, Stage::Sprout);
    assert_eq!(garden.plots()[0].plant().unwrap().stage_gp, 25);

    // Tick 6: Sprout -> Young, carry-over = 5 GP.
    clock.advance_ticks(1);
    let out = garden.tick(TickInput::idle(), &balance);
    assert_eq!(
        out.transitions,
        vec![StageTransition {
            plot_index: 0,
            from: Stage::Sprout,
            to: Stage::Young,
        }]
    );
    let p = garden.plots()[0].plant().unwrap();
    assert_eq!(p.stage, Stage::Young);
    assert_eq!(p.stage_gp, 5);
    assert_eq!(p.total_gp, 60);

    // Ticks 7..=8: stays Young (25 / 35 GP).
    for _ in 0..2 {
        clock.advance_ticks(1);
        assert!(garden
            .tick(TickInput::idle(), &balance)
            .transitions
            .is_empty());
    }
    assert_eq!(garden.plots()[0].plant().unwrap().stage, Stage::Young);
    assert_eq!(garden.plots()[0].plant().unwrap().stage_gp, 25);

    // Tick 9: Young -> Mature at exact boundary (35 / 35 GP -> carry-over 0).
    clock.advance_ticks(1);
    let out = garden.tick(TickInput::idle(), &balance);
    assert_eq!(
        out.transitions,
        vec![StageTransition {
            plot_index: 0,
            from: Stage::Young,
            to: Stage::Mature,
        }]
    );
    let p = garden.plots()[0].plant().unwrap();
    assert_eq!(p.stage, Stage::Mature);
    assert_eq!(p.stage_gp, 0);
    assert_eq!(p.total_gp, 90);
    assert_eq!(p.bloomed_at, None);

    // Tick 10: stays Mature (10 / 20 GP).
    clock.advance_ticks(1);
    assert!(garden
        .tick(TickInput::idle(), &balance)
        .transitions
        .is_empty());
    assert_eq!(garden.plots()[0].plant().unwrap().stage, Stage::Mature);

    // Tick 11 (pre-tick index = 10): Mature -> Blooming!
    clock.advance_ticks(1);
    let expected_bloom_seed = rng_for_stream(0xCAFE_BABE, 10, 1).next_u64();
    let out = garden.tick(TickInput::idle(), &balance);
    assert_eq!(
        out.transitions,
        vec![StageTransition {
            plot_index: 0,
            from: Stage::Mature,
            to: Stage::Blooming,
        }]
    );
    assert_eq!(garden.last_tick_at(), clock.now());

    let p = garden.plots()[0].plant().unwrap();
    assert_eq!(p.stage, Stage::Blooming);
    assert!(p.is_blooming());
    assert_eq!(p.stage_gp, 0);
    assert_eq!(p.total_gp, 110);
    assert_eq!(
        p.bloomed_at,
        Some(Timestamp::from_unix_secs(1_700_000_000 + 11 * TICK_SECS))
    );
    assert_eq!(p.bloom_seed, Some(expected_bloom_seed));

    // Tick 12: Blooming stays Blooming, bloomed_at and bloom_seed never get overwritten,
    // and surplus GP continues accumulating monotonically.
    clock.advance_ticks(1);
    let out = garden.tick(TickInput::active(), &balance);
    assert!(out.transitions.is_empty());
    let p = garden.plots()[0].plant().unwrap();
    assert_eq!(p.stage, Stage::Blooming);
    assert_eq!(p.stage_gp, 25); // base(10) + sun(15)
    assert_eq!(p.total_gp, 135);
    assert_eq!(
        p.bloomed_at,
        Some(Timestamp::from_unix_secs(1_700_000_000 + 11 * TICK_SECS))
    );
    assert_eq!(p.bloom_seed, Some(expected_bloom_seed));
}

#[test]
fn large_tick_gp_carries_across_multiple_stages_and_applies_species_and_season_multipliers() {
    let balance = small_balance([20, 20, 20, 20], 7);
    let mut profiles = BTreeMap::new();
    // Species multiplier = 1.5x (15_000 bp)
    profiles.insert(
        "fern".to_string(),
        GrowthProfile::new([20, 20, 20, 20], 15_000).unwrap(),
    );

    let mut garden = Garden::new(77, 2, Timestamp::from_unix_secs(1_000));
    garden.plant_seed(0, "fern").unwrap();

    // raw = base(10) + sun(15) + water(15) = 40
    // scaled = 40 * 1.5 (species) * 1.25 (season = 12_500 bp) = 75 GP in one tick!
    // Thresholds [20, 20, 20, 20] -> crosses Seed->Sprout (20), Sprout->Young (40), Young->Mature (60),
    // landing in Mature with stage_gp = 15, total_gp = 75.
    let input = TickInput::with_water(true, 15).with_season_multiplier_bp(12_500);
    let out = garden.tick_with_profiles(input, &balance, &(&balance, &profiles));

    assert_eq!(
        out.transitions,
        vec![
            StageTransition {
                plot_index: 0,
                from: Stage::Seed,
                to: Stage::Sprout,
            },
            StageTransition {
                plot_index: 0,
                from: Stage::Sprout,
                to: Stage::Young,
            },
            StageTransition {
                plot_index: 0,
                from: Stage::Young,
                to: Stage::Mature,
            },
        ]
    );
    let p = garden.plots()[0].plant().unwrap();
    assert_eq!(p.stage, Stage::Mature);
    assert_eq!(p.stage_gp, 15);
    assert_eq!(p.total_gp, 75);
}

#[test]
fn dormancy_pauses_after_configured_days_preserves_plants_and_wakes_on_activity() {
    // Use 1 day = 1_440 ticks so we test exact multi-hundred-tick day boundaries cleanly,
    // plus verify default 7-day boundary behavior.
    let balance = small_balance([50_000, 50_000, 50_000, 50_000], 1);
    let dormant_ticks = balance.dormant_after_ticks();
    assert_eq!(dormant_ticks, TICKS_PER_DAY);

    let mut garden = Garden::new(1234, 3, Timestamp::from_unix_secs(1_700_000_000));
    garden.plant_seed(0, "chamomile").unwrap();
    garden.plant_seed(2, "clover").unwrap();

    // Ticks 1..=(TICKS_PER_DAY - 1): not dormant yet, grows by base (10 GP/tick).
    garden.tick_n(dormant_ticks - 1, TickInput::idle(), &balance);
    assert!(!garden.is_dormant());
    assert_eq!(garden.inactive_ticks(), dormant_ticks - 1);
    assert_eq!(
        garden.plots()[0].plant().unwrap().total_gp,
        (dormant_ticks - 1) * 10
    );

    // Tick TICKS_PER_DAY completes 1.0 full day of inactivity: grows on this final tick,
    // and transitions the garden into dormancy.
    let enter_outcome = garden.tick(TickInput::idle(), &balance);
    assert!(enter_outcome.dormant);
    assert!(garden.is_dormant());
    assert_eq!(garden.inactive_ticks(), dormant_ticks);
    let gp_at_dormancy = dormant_ticks * 10;
    assert_eq!(garden.plots()[0].plant().unwrap().total_gp, gp_at_dormancy);
    assert_eq!(garden.plots()[2].plant().unwrap().total_gp, gp_at_dormancy);

    // 5 more days of complete neglect while dormant: zero GP gained, nothing lost or deleted.
    garden.tick_n(5 * TICKS_PER_DAY, TickInput::idle(), &balance);
    assert!(garden.is_dormant());
    assert_eq!(garden.inactive_ticks(), 6 * TICKS_PER_DAY);
    assert_eq!(garden.plots()[0].plant().unwrap().total_gp, gp_at_dormancy);
    assert_eq!(garden.plots()[2].plant().unwrap().total_gp, gp_at_dormancy);

    // User returns (active tick): wakes immediately and applies base(10) + sun(15) = 25 GP!
    let wake_outcome = garden.tick(TickInput::active(), &balance);
    assert!(!wake_outcome.dormant);
    assert!(!garden.is_dormant());
    assert_eq!(garden.inactive_ticks(), 0);
    assert_eq!(
        garden.plots()[0].plant().unwrap().total_gp,
        gp_at_dormancy + 25
    );
    assert_eq!(
        garden.plots()[2].plant().unwrap().total_gp,
        gp_at_dormancy + 25
    );

    // Another full day of inactivity puts the garden back into dormancy without losing the +25 GP.
    garden.tick_n(dormant_ticks, TickInput::idle(), &balance);
    assert!(garden.is_dormant());
    let second_dormancy_gp = gp_at_dormancy + 25 + dormant_ticks * 10;
    garden.tick_n(100, TickInput::idle(), &balance);
    assert_eq!(
        garden.plots()[0].plant().unwrap().total_gp,
        second_dormancy_gp
    );
}

#[test]
fn default_seven_day_dormancy_allows_five_day_common_plant_to_bloom_on_time_alone() {
    let balance = Balance::load_default();
    let mut garden = Garden::from_balance(99, &balance, Timestamp::from_unix_secs(1_700_000_000));
    garden.plant_seed(0, "daisy").unwrap();

    // 5 days of pure idle wall time (no signals enabled): blooms before 7-day dormancy.
    garden.tick_n(5 * TICKS_PER_DAY, TickInput::idle(), &balance);
    assert!(!garden.is_dormant());
    assert_eq!(garden.plots()[0].plant().unwrap().stage, Stage::Blooming);

    // 2 more idle days reach the 7-day dormancy mark.
    garden.tick_n(2 * TICKS_PER_DAY, TickInput::idle(), &balance);
    assert!(garden.is_dormant());
}

#[test]
fn per_tick_seeding_matches_rng_factory_and_separates_plots() {
    let balance = small_balance([5, 5, 5, 5], 7);
    let seed = 0x1234_5678_9ABC_DEF0;
    let mut garden = Garden::new(seed, 3, Timestamp::from_unix_secs(0));
    garden.plant_seed(0, "iris").unwrap();
    garden.plant_seed(2, "lily").unwrap();

    // Tick 0 gives 25 GP (active: 10 + 15), blooming both plots on tick_index 0.
    let out0 = garden.tick(TickInput::active(), &balance);
    assert_eq!(out0.tick_index, 0);
    assert_eq!(out0.tick_roll, rng_for_tick(seed, 0).next_u64());

    let bloom0 = garden.plots()[0].plant().unwrap().bloom_seed.unwrap();
    let bloom2 = garden.plots()[2].plant().unwrap().bloom_seed.unwrap();
    assert_eq!(bloom0, rng_for_stream(seed, 0, 1).next_u64());
    assert_eq!(bloom2, rng_for_stream(seed, 0, 3).next_u64());
    assert_ne!(bloom0, bloom2);

    let out1 = garden.tick(TickInput::idle(), &balance);
    assert_eq!(out1.tick_index, 1);
    assert_eq!(out1.tick_roll, rng_for_tick(seed, 1).next_u64());
    assert_ne!(out0.tick_roll, out1.tick_roll);
}

#[test]
fn plot_management_enforces_bounds_and_supports_move_and_clear() {
    let mut garden = Garden::new(1, 2, Timestamp::from_unix_secs(500));
    assert_eq!(
        garden.plant_seed(2, "moss"),
        Err(SimError::PlotOutOfBounds {
            plot_index: 2,
            plot_count: 2,
        })
    );

    garden.plant_seed(0, "moss").unwrap();
    assert_eq!(
        garden.plant_seed(0, "fern"),
        Err(SimError::PlotOccupied { plot_index: 0 })
    );

    garden.move_plant(0, 1).unwrap();
    assert!(garden.plots()[0].is_empty());
    assert_eq!(garden.plots()[1].plant().unwrap().species_id, "moss");

    assert_eq!(
        garden.move_plant(0, 1),
        Err(SimError::PlotEmpty { plot_index: 0 })
    );

    let removed = garden.clear_plot(1).unwrap();
    assert_eq!(removed.species_id, "moss");
    assert!(garden.plots()[1].is_empty());
    assert_eq!(
        garden.clear_plot(1),
        Err(SimError::PlotEmpty { plot_index: 1 })
    );
}
