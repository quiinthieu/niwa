//! Weighted seed drop table driven by `data/balance.toml` rarity and season weights (PLAN §4.2 & T04).

use crate::balance::DropBalance;
use crate::rng::GardenRng;
use crate::season::Season;
use crate::species::{Rarity, Species, SpeciesCatalog};
use rand::RngCore;
use thiserror::Error;

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum DropTableError {
    #[error("cannot sample seed drop from an empty or zero-weight species pool")]
    EmptyCandidatePool,
}

/// Unbiased uniform integer in `0..upper` derived purely from `Pcg64::next_u64`
/// so sampling sequences stay bit-for-bit stable across platforms and `rand` point releases.
fn uniform_below(rng: &mut GardenRng, upper: u64) -> u64 {
    debug_assert!(upper > 0);
    let zone = u64::MAX - (u64::MAX % upper);
    loop {
        let draw = rng.next_u64();
        if draw < zone {
            return draw % upper;
        }
    }
}

/// Deterministic weighted seed sampler for blooming harvests and same-rarity composting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DropTable {
    weights: DropBalance,
}

impl DropTable {
    #[must_use]
    pub const fn new(weights: DropBalance) -> Self {
        Self { weights }
    }

    #[must_use]
    pub const fn weights(&self) -> &DropBalance {
        &self.weights
    }

    #[must_use]
    pub fn candidate_weight(&self, species: &Species, season: Season) -> u64 {
        let rarity_w = self.weights.rarity_weight(species.rarity);
        let season_w = self
            .weights
            .season_weight_multiplier(species.prefers_season(season));
        rarity_w.saturating_mul(season_w)
    }

    fn sample_filtered<'a, P>(
        &self,
        catalog: &'a SpeciesCatalog,
        season: Season,
        rng: &mut GardenRng,
        predicate: P,
    ) -> Result<&'a Species, DropTableError>
    where
        P: Fn(&Species) -> bool,
    {
        let mut weighted: Vec<(&'a Species, u64)> = Vec::new();
        let mut total_weight: u64 = 0;

        for species in catalog.iter() {
            if !predicate(species) {
                continue;
            }
            let w = self.candidate_weight(species, season);
            if w > 0 {
                total_weight = total_weight.saturating_add(w);
                weighted.push((species, w));
            }
        }

        if total_weight == 0 || weighted.is_empty() {
            return Err(DropTableError::EmptyCandidatePool);
        }

        let mut cursor = uniform_below(rng, total_weight);
        for (species, w) in &weighted {
            if cursor < *w {
                return Ok(*species);
            }
            cursor -= *w;
        }

        Ok(weighted.last().expect("weighted pool is non-empty").0)
    }

    /// Samples a single seed drop weighted by rarity and active `season`.
    pub fn sample_harvest<'a>(
        &self,
        catalog: &'a SpeciesCatalog,
        season: Season,
        rng: &mut GardenRng,
    ) -> Result<&'a Species, DropTableError> {
        self.sample_filtered(catalog, season, rng, |_| true)
    }

    /// Samples `seeds_per_harvest` seed drops for harvesting a blooming plant.
    pub fn sample_harvest_batch<'a>(
        &self,
        catalog: &'a SpeciesCatalog,
        season: Season,
        rng: &mut GardenRng,
    ) -> Result<Vec<&'a Species>, DropTableError> {
        let count = self.weights.seeds_per_harvest;
        let mut out = Vec::with_capacity(count);
        for _ in 0..count {
            out.push(self.sample_harvest(catalog, season, rng)?);
        }
        Ok(out)
    }

    /// Samples a seed of the exact `rarity` tier weighted by seasonal preference
    /// (used when composting a plant per PLAN §4.2).
    pub fn sample_same_rarity<'a>(
        &self,
        catalog: &'a SpeciesCatalog,
        rarity: Rarity,
        season: Season,
        rng: &mut GardenRng,
    ) -> Result<&'a Species, DropTableError> {
        self.sample_filtered(catalog, season, rng, |sp| sp.rarity == rarity)
    }
}
