//! Deterministic seeded PRNG factory for `tg-core`.
//!
//! Per PLAN §4.1, simulation randomness is derived purely from
//! `(garden_seed, tick_index)` so batched catch-up and step-by-step ticks
//! produce identical outcomes across platforms.

use rand::SeedableRng;
use rand_pcg::Pcg64;

/// Portable, deterministic PCG64 generator used across `tg-core`.
pub type GardenRng = Pcg64;

const GOLDEN_GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;

const fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(GOLDEN_GAMMA);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// Creates a deterministic [`GardenRng`] from a single 64-bit seed.
#[must_use]
pub fn rng_from_seed(seed: u64) -> GardenRng {
    Pcg64::seed_from_u64(seed)
}

/// Creates a deterministic [`GardenRng`] for `(garden_seed, tick_index)` (stream 0).
#[must_use]
pub fn rng_for_tick(garden_seed: u64, tick_index: u64) -> GardenRng {
    rng_for_stream(garden_seed, tick_index, 0)
}

/// Creates a deterministic [`GardenRng`] for `(garden_seed, tick_index, stream)`.
///
/// Mixes all three 64-bit inputs through SplitMix64 into a little-endian 256-bit
/// PCG64 seed so distinct tuples do not collide and results are endian-portable.
#[must_use]
pub fn rng_for_stream(garden_seed: u64, tick_index: u64, stream: u64) -> GardenRng {
    let w0 = splitmix64(garden_seed ^ 0x4E49_5741_5345_4544);
    let w1 = splitmix64(tick_index ^ w0.rotate_left(17));
    let w2 = splitmix64(stream ^ w1.rotate_left(29));
    let w3 = splitmix64(garden_seed.wrapping_add(tick_index).wrapping_add(stream) ^ w2);

    let mut seed_bytes = [0_u8; 32];
    seed_bytes[0..8].copy_from_slice(&w0.to_le_bytes());
    seed_bytes[8..16].copy_from_slice(&w1.to_le_bytes());
    seed_bytes[16..24].copy_from_slice(&w2.to_le_bytes());
    seed_bytes[24..32].copy_from_slice(&w3.to_le_bytes());

    Pcg64::from_seed(seed_bytes)
}
