use proptest::prelude::*;
use std::collections::BTreeMap;
use tg_core::balance::{Balance, DropBalance, GrowthProfile, SeasonBalance, MULTIPLIER_BP_SCALE};
use tg_core::clock::Timestamp;
use tg_core::sim::{Garden, TickInput};

fn arb_thresholds() -> impl Strategy<Value = [u64; 4]> {
    [1_u64..=500, 1_u64..=500, 1_u64..=500, 1_u64..=500]
}

fn arb_profile() -> impl Strategy<Value = GrowthProfile> {
    (arb_thresholds(), 2_500_u32..=30_000_u32)
        .prop_map(|(thresholds, species_bp)| GrowthProfile::new(thresholds, species_bp).unwrap())
}

fn arb_tick_input() -> impl Strategy<Value = TickInput> {
    (
        any::<bool>(),
        0_u64..=200,
        prop::option::of(2_500_u32..=25_000_u32),
    )
        .prop_map(|(active, water_gp, season_bp)| {
            let base = TickInput::with_water(active, water_gp);
            match season_bp {
                Some(bp) => base.with_season_multiplier_bp(bp),
                None => base,
            }
        })
}

fn build_seeded_garden(seed: u64, plot_species: &[Option<u8>], start_secs: i64) -> Garden {
    let species_names = ["alpha", "beta", "gamma"];
    let mut garden = Garden::new(
        seed,
        plot_species.len(),
        Timestamp::from_unix_secs(start_secs),
    );
    for (idx, slot) in plot_species.iter().copied().enumerate() {
        if let Some(choice) = slot {
            let name = species_names[usize::from(choice) % species_names.len()];
            garden.plant_seed(idx, name).expect("plant initial seed");
        }
    }
    garden
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 256,
        failure_persistence: None,
        .. ProptestConfig::default()
    })]

    #[test]
    fn batching_invariance_holds_across_arbitrary_event_slices(
        seed in any::<u64>(),
        base_gp in 1_u64..=250,
        sun_gp in 0_u64..=300,
        dormant_after_days in 0_u32..=2,
        default_profile in arb_profile(),
        alpha_profile in arb_profile(),
        beta_profile in arb_profile(),
        gamma_profile in arb_profile(),
        plot_species in prop::collection::vec(prop::option::of(0_u8..3), 1..=6),
        inputs in prop::collection::vec(arb_tick_input(), 0..=220),
        raw_splits in prop::collection::vec(0_usize..=220, 0..=16),
    ) {
        let balance = Balance {
            default_plots: plot_species.len(),
            base_gp_per_tick: base_gp,
            sun_gp_per_tick: sun_gp,
            default_profile,
            default_season_multiplier_bp: MULTIPLIER_BP_SCALE,
            dormant_after_days,
            seasons: SeasonBalance::default(),
            drops: DropBalance::default(),
        };

        let mut profiles = BTreeMap::new();
        profiles.insert("alpha".to_string(), alpha_profile);
        profiles.insert("beta".to_string(), beta_profile);
        profiles.insert("gamma".to_string(), gamma_profile);
        let resolver = (&balance, &profiles);

        let mut garden_single_steps = build_seeded_garden(seed, &plot_species, 1_700_000_000);
        let mut garden_one_batch = garden_single_steps.clone();
        let mut garden_chunked = garden_single_steps.clone();

        // 1. Step-by-step single tick() calls.
        let mut outcomes_step = Vec::with_capacity(inputs.len());
        for &input in &inputs {
            outcomes_step.push(garden_single_steps.tick_with_profiles(input, &balance, &resolver));
        }

        // 2. Single full-slice simulate_ticks call.
        let outcomes_all = garden_one_batch.simulate_ticks_with_profiles(&inputs, &balance, &resolver);

        // 3. Arbitrary partitioned sub-slices (including empty slices).
        let mut split_points: Vec<usize> = raw_splits
            .into_iter()
            .map(|idx| idx.min(inputs.len()))
            .collect();
        split_points.sort_unstable();

        let mut outcomes_chunked = Vec::with_capacity(inputs.len());
        let mut cursor = 0;
        for split in split_points {
            let chunk = &inputs[cursor..split];
            outcomes_chunked.extend(
                garden_chunked.simulate_ticks_with_profiles(chunk, &balance, &resolver),
            );
            cursor = split;
        }
        outcomes_chunked.extend(
            garden_chunked.simulate_ticks_with_profiles(&inputs[cursor..], &balance, &resolver),
        );

        prop_assert_eq!(&garden_single_steps, &garden_one_batch);
        prop_assert_eq!(&garden_single_steps, &garden_chunked);
        prop_assert_eq!(&outcomes_step, &outcomes_all);
        prop_assert_eq!(&outcomes_step, &outcomes_chunked);
    }

    #[test]
    fn uniform_tick_n_batching_is_invariant_across_multi_day_dormancy_boundaries(
        seed in any::<u64>(),
        base_gp in 1_u64..=200,
        sun_gp in 0_u64..=200,
        dormant_after_days in 0_u32..=2,
        profile in arb_profile(),
        plot_species in prop::collection::vec(prop::option::of(0_u8..3), 1..=5),
        input in arb_tick_input(),
        chunks in prop::collection::vec(0_u64..=600, 1..=10),
    ) {
        let balance = Balance {
            default_plots: plot_species.len(),
            base_gp_per_tick: base_gp,
            sun_gp_per_tick: sun_gp,
            default_profile: profile,
            default_season_multiplier_bp: MULTIPLIER_BP_SCALE,
            dormant_after_days,
            seasons: SeasonBalance::default(),
            drops: DropBalance::default(),
        };

        let total_ticks: u64 = chunks.iter().copied().sum();
        let mut garden_all = build_seeded_garden(seed, &plot_species, 1_700_000_000);
        let mut garden_chunks = garden_all.clone();

        garden_all.tick_n(total_ticks, input, &balance);
        for chunk in chunks {
            garden_chunks.tick_n(chunk, input, &balance);
        }

        prop_assert_eq!(garden_all, garden_chunks);
    }

    #[test]
    fn monotonicity_and_non_loss_hold_on_every_tick(
        seed in any::<u64>(),
        base_gp in 0_u64..=250,
        sun_gp in 0_u64..=300,
        dormant_after_days in 0_u32..=2,
        default_profile in arb_profile(),
        alpha_profile in arb_profile(),
        beta_profile in arb_profile(),
        gamma_profile in arb_profile(),
        plot_species in prop::collection::vec(prop::option::of(0_u8..3), 1..=6),
        inputs in prop::collection::vec(arb_tick_input(), 1..=200),
    ) {
        let balance = Balance {
            default_plots: plot_species.len(),
            base_gp_per_tick: base_gp,
            sun_gp_per_tick: sun_gp,
            default_profile,
            default_season_multiplier_bp: MULTIPLIER_BP_SCALE,
            dormant_after_days,
            seasons: SeasonBalance::default(),
            drops: DropBalance::default(),
        };

        let mut profiles = BTreeMap::new();
        profiles.insert("alpha".to_string(), alpha_profile);
        profiles.insert("beta".to_string(), beta_profile);
        profiles.insert("gamma".to_string(), gamma_profile);
        let resolver = (&balance, &profiles);

        let mut garden = build_seeded_garden(seed, &plot_species, 1_700_000_000);

        for input in inputs {
            let prev = garden.clone();
            garden.tick_with_profiles(input, &balance, &resolver);

            prop_assert_eq!(garden.tick_index(), prev.tick_index() + 1);
            prop_assert!(garden.last_tick_at() >= prev.last_tick_at());
            prop_assert_eq!(garden.plots().len(), prev.plots().len());

            for (prev_plot, curr_plot) in prev.plots().iter().zip(garden.plots().iter()) {
                match (prev_plot.plant(), curr_plot.plant()) {
                    (None, None) => {}
                    (None, Some(_)) => {
                        prop_assert!(false, "empty plot unexpectedly gained a plant during tick");
                    }
                    (Some(_), None) => {
                        prop_assert!(false, "time passing must never delete a plant");
                    }
                    (Some(prev_plant), Some(curr_plant)) => {
                        prop_assert_eq!(&curr_plant.species_id, &prev_plant.species_id);
                        prop_assert_eq!(curr_plant.planted_at, prev_plant.planted_at);
                        prop_assert_eq!(curr_plant.planted_tick, prev_plant.planted_tick);

                        // Monotonic total GP and stage progression.
                        prop_assert!(curr_plant.total_gp >= prev_plant.total_gp);
                        prop_assert!(curr_plant.stage >= prev_plant.stage);
                        if curr_plant.stage == prev_plant.stage {
                            prop_assert!(curr_plant.stage_gp >= prev_plant.stage_gp);
                        }

                        // Bloom metadata, once recorded, is immutable.
                        if let Some(prev_bloom_at) = prev_plant.bloomed_at {
                            prop_assert_eq!(curr_plant.bloomed_at, Some(prev_bloom_at));
                        }
                        if let Some(prev_bloom_seed) = prev_plant.bloom_seed {
                            prop_assert_eq!(curr_plant.bloom_seed, Some(prev_bloom_seed));
                        }

                        // Exact conservation of GP across completed stages + current stage_gp.
                        let profile = tg_core::sim::ProfileResolver::profile_for(
                            &resolver,
                            &curr_plant.species_id,
                        );
                        let expected_total = profile
                            .completed_thresholds_before(curr_plant.stage)
                            .saturating_add(curr_plant.stage_gp);
                        prop_assert_eq!(curr_plant.total_gp, expected_total);
                    }
                }
            }
        }
    }
}
