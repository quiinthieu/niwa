use rand::Rng;
use tg_core::clock::{Clock, FakeClock, Timestamp, TICK_SECS};
use tg_core::rng::{rng_for_stream, rng_for_tick, rng_from_seed};

#[test]
fn timestamp_and_tick_arithmetic() {
    assert_eq!(TICK_SECS, 60);

    let start = Timestamp::from_unix_secs(1_700_000_000);
    assert_eq!(start.as_unix_secs(), 1_700_000_000);
    assert_eq!(start.add_secs(15).as_unix_secs(), 1_700_000_015);
    assert_eq!(start.add_secs(-30).as_unix_secs(), 1_699_999_970);
    assert_eq!(start.add_ticks(3).as_unix_secs(), 1_700_000_180);
}

#[test]
fn fake_clock_advances_and_supports_trait_injection() {
    let clock = FakeClock::from_unix_secs(1_700_000_000);

    fn read_via_trait(clock: &dyn Clock) -> Timestamp {
        clock.now()
    }

    assert_eq!(read_via_trait(&clock).as_unix_secs(), 1_700_000_000);
    assert_eq!(clock.advance_secs(45).as_unix_secs(), 1_700_000_045);
    assert_eq!(read_via_trait(&&clock).as_unix_secs(), 1_700_000_045);

    // Supports advancing by whole 60 s garden ticks.
    assert_eq!(clock.advance_ticks(2).as_unix_secs(), 1_700_000_165);

    // Supports backwards clock-skew simulation for catch-up tests (T06).
    assert_eq!(clock.advance_secs(-65).as_unix_secs(), 1_700_000_100);

    clock.set(Timestamp::from_unix_secs(1_700_001_000));
    assert_eq!(clock.now().as_unix_secs(), 1_700_001_000);
}

#[test]
fn seeded_rng_is_deterministic_and_batch_invariant() {
    let mut a = rng_from_seed(0xC0FFEE);
    let mut b = rng_from_seed(0xC0FFEE);
    let seq_a: [u64; 8] = core::array::from_fn(|_| a.random());
    let seq_b: [u64; 8] = core::array::from_fn(|_| b.random());
    assert_eq!(seq_a, seq_b);

    // Per-tick seeding depends only on (garden_seed, tick_index), so jumping
    // directly to tick 42 equals stepping ticks 0..=42 one at a time.
    let garden_seed = 0x4E49_5741_2026_0001;
    let mut stepped_tick_42 = 0_u64;
    for tick in 0..=42 {
        let mut rng = rng_for_tick(garden_seed, tick);
        let draw: u64 = rng.random();
        if tick == 42 {
            stepped_tick_42 = draw;
        }
    }
    let direct_tick_42: u64 = rng_for_tick(garden_seed, 42).random();
    assert_eq!(stepped_tick_42, direct_tick_42);
}

#[test]
fn seeded_rng_separates_seeds_ticks_and_streams() {
    // Guard against naive XOR seeding where (1, 1) == (2, 2) or (1, 2) == (2, 1).
    let draw = |seed, tick| rng_for_tick(seed, tick).random::<u64>();
    let samples = [draw(1, 1), draw(2, 2), draw(1, 2), draw(2, 1)];
    for i in 0..samples.len() {
        for j in (i + 1)..samples.len() {
            assert_ne!(samples[i], samples[j], "collision between {i} and {j}");
        }
    }

    let s0: u64 = rng_for_stream(42, 7, 0).random();
    let s1: u64 = rng_for_stream(42, 7, 1).random();
    assert_ne!(s0, s1);
    assert_eq!(s0, rng_for_tick(42, 7).random::<u64>());
}

fn sample_tick_trace(garden_seed: u64, start_secs: i64, ticks: u64) -> String {
    let clock = FakeClock::from_unix_secs(start_secs);
    let mut lines = Vec::new();
    for tick in 0..ticks {
        let ts = if tick == 0 {
            clock.now()
        } else {
            clock.advance_ticks(1)
        };
        let mut rng = rng_for_tick(garden_seed, tick);
        let roll: u32 = rng.random_range(0..10_000);
        lines.push(format!(
            "tick={tick} ts={} roll={roll:04}",
            ts.as_unix_secs()
        ));
    }
    lines.join("\n")
}

#[test]
fn sample_clock_and_rng_pass_100x_in_a_loop() {
    let baseline = sample_tick_trace(0xDEC0_DED0_1234_5678, 1_700_000_000, 6);
    for iteration in 0..100 {
        let current = sample_tick_trace(0xDEC0_DED0_1234_5678, 1_700_000_000, 6);
        assert_eq!(current, baseline, "mismatch at loop iteration {iteration}");
    }
}

#[test]
fn seeded_tick_trace_matches_golden_snapshot() {
    let trace = sample_tick_trace(0xDEC0_DED0_1234_5678, 1_700_000_000, 6);
    insta::assert_snapshot!("seeded_tick_trace", trace);
}
