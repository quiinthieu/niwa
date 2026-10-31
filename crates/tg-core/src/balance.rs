//! Tunable gameplay balance parameters loaded from `data/balance.toml` (PLAN §0.9 & §4).

use crate::clock::TICKS_PER_DAY;
use crate::model::Stage;
use crate::species::Rarity;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Basis-point denominator representing a `1.0x` multiplier (`10_000 == 1.0`).
pub const MULTIPLIER_BP_SCALE: u32 = 10_000;

/// Embedded default `data/balance.toml` (pure compile-time string, no runtime I/O).
pub const DEFAULT_BALANCE_TOML: &str = include_str!("../../../data/balance.toml");

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum BalanceError {
    #[error("failed to parse balance TOML: {0}")]
    ParseToml(String),
    #[error("default_plots must be at least 1")]
    ZeroDefaultPlots,
    #[error("season_length_days must be at least 1")]
    ZeroSeasonLengthDays,
    #[error("seeds_per_harvest must be at least 1")]
    ZeroSeedsPerHarvest,
    #[error("drop table weights must include at least one positive rarity and season weight")]
    ZeroDropWeights,
    #[error("stage threshold at index {stage_index} must be >= 1, got {value}")]
    InvalidStageThreshold { stage_index: usize, value: u64 },
    #[error(
        "invalid multiplier for '{field}': must be finite, non-negative, and fit basis points"
    )]
    InvalidMultiplier { field: &'static str },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBalance {
    garden: RawGardenSection,
    growth: RawGrowthSection,
    dormancy: RawDormancySection,
    #[serde(default)]
    seasons: RawSeasonsSection,
    #[serde(default)]
    drops: RawDropsSection,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGardenSection {
    default_plots: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGrowthSection {
    base_gp_per_tick: u64,
    sun_gp_per_tick: u64,
    default_stage_thresholds: [u64; 4],
    default_species_multiplier: f64,
    default_season_multiplier: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDormancySection {
    dormant_after_days: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSeasonsSection {
    season_length_days: u32,
    preferred_season_multiplier: f64,
    off_season_multiplier: f64,
}

impl Default for RawSeasonsSection {
    fn default() -> Self {
        Self {
            season_length_days: 7,
            preferred_season_multiplier: 1.25,
            off_season_multiplier: 0.85,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDropsSection {
    common_weight: u64,
    uncommon_weight: u64,
    rare_weight: u64,
    mythic_weight: u64,
    in_season_weight_multiplier: u64,
    off_season_weight_multiplier: u64,
    seeds_per_harvest: usize,
}

impl Default for RawDropsSection {
    fn default() -> Self {
        Self {
            common_weight: 600,
            uncommon_weight: 270,
            rare_weight: 100,
            mythic_weight: 30,
            in_season_weight_multiplier: 3,
            off_season_weight_multiplier: 1,
            seeds_per_harvest: 1,
        }
    }
}

/// Validated seasonal cycle length and growth multipliers (PLAN §4.2).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeasonBalance {
    pub season_length_days: u32,
    pub preferred_season_multiplier_bp: u32,
    pub off_season_multiplier_bp: u32,
}

impl SeasonBalance {
    #[must_use]
    pub const fn multiplier_bp_for(&self, in_preferred_season: bool) -> u32 {
        if in_preferred_season {
            self.preferred_season_multiplier_bp
        } else {
            self.off_season_multiplier_bp
        }
    }
}

impl Default for SeasonBalance {
    fn default() -> Self {
        Self {
            season_length_days: 7,
            preferred_season_multiplier_bp: 12_500,
            off_season_multiplier_bp: 8_500,
        }
    }
}

/// Validated seed drop-table weights by rarity and season (PLAN §4.2).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DropBalance {
    pub common_weight: u64,
    pub uncommon_weight: u64,
    pub rare_weight: u64,
    pub mythic_weight: u64,
    pub in_season_weight_multiplier: u64,
    pub off_season_weight_multiplier: u64,
    pub seeds_per_harvest: usize,
}

impl DropBalance {
    #[must_use]
    pub const fn rarity_weight(&self, rarity: Rarity) -> u64 {
        match rarity {
            Rarity::Common => self.common_weight,
            Rarity::Uncommon => self.uncommon_weight,
            Rarity::Rare => self.rare_weight,
            Rarity::Mythic => self.mythic_weight,
        }
    }

    #[must_use]
    pub const fn season_weight_multiplier(&self, in_preferred_season: bool) -> u64 {
        if in_preferred_season {
            self.in_season_weight_multiplier
        } else {
            self.off_season_weight_multiplier
        }
    }
}

impl Default for DropBalance {
    fn default() -> Self {
        Self {
            common_weight: 600,
            uncommon_weight: 270,
            rare_weight: 100,
            mythic_weight: 30,
            in_season_weight_multiplier: 3,
            off_season_weight_multiplier: 1,
            seeds_per_harvest: 1,
        }
    }
}

/// Converts a floating-point TOML multiplier into deterministic basis points (`10_000 == 1.0x`).
pub fn multiplier_to_bp(field: &'static str, multiplier: f64) -> Result<u32, BalanceError> {
    if !multiplier.is_finite() || multiplier < 0.0 {
        return Err(BalanceError::InvalidMultiplier { field });
    }
    let scaled = (multiplier * f64::from(MULTIPLIER_BP_SCALE)).round();
    if !scaled.is_finite() || scaled < 0.0 || scaled > f64::from(u32::MAX) {
        return Err(BalanceError::InvalidMultiplier { field });
    }
    Ok(scaled as u32)
}

/// Growth thresholds and species multiplier governing a plant's progression.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrowthProfile {
    pub stage_thresholds: [u64; 4],
    pub species_multiplier_bp: u32,
}

impl GrowthProfile {
    pub fn new(
        stage_thresholds: [u64; 4],
        species_multiplier_bp: u32,
    ) -> Result<Self, BalanceError> {
        for (stage_index, value) in stage_thresholds.iter().copied().enumerate() {
            if value == 0 {
                return Err(BalanceError::InvalidStageThreshold { stage_index, value });
            }
        }
        Ok(Self {
            stage_thresholds,
            species_multiplier_bp,
        })
    }

    #[must_use]
    pub fn total_to_bloom(&self) -> u64 {
        self.stage_thresholds
            .iter()
            .copied()
            .fold(0_u64, u64::saturating_add)
    }

    #[must_use]
    pub fn completed_thresholds_before(&self, stage: Stage) -> u64 {
        let count = match stage {
            Stage::Seed => 0,
            Stage::Sprout => 1,
            Stage::Young => 2,
            Stage::Mature => 3,
            Stage::Blooming => 4,
        };
        self.stage_thresholds[..count]
            .iter()
            .copied()
            .fold(0_u64, u64::saturating_add)
    }
}

/// Validated gameplay balance configuration for `tg-core`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Balance {
    pub default_plots: usize,
    pub base_gp_per_tick: u64,
    pub sun_gp_per_tick: u64,
    pub default_profile: GrowthProfile,
    pub default_season_multiplier_bp: u32,
    pub dormant_after_days: u32,
    pub seasons: SeasonBalance,
    pub drops: DropBalance,
}

impl Balance {
    #[must_use]
    pub fn load_default() -> Self {
        Self::from_toml_str(DEFAULT_BALANCE_TOML)
            .expect("embedded data/balance.toml must parse and validate")
    }

    pub fn from_toml_str(raw: &str) -> Result<Self, BalanceError> {
        let parsed: RawBalance =
            toml::from_str(raw).map_err(|err| BalanceError::ParseToml(err.to_string()))?;

        if parsed.garden.default_plots == 0 {
            return Err(BalanceError::ZeroDefaultPlots);
        }
        if parsed.seasons.season_length_days == 0 {
            return Err(BalanceError::ZeroSeasonLengthDays);
        }
        if parsed.drops.seeds_per_harvest == 0 {
            return Err(BalanceError::ZeroSeedsPerHarvest);
        }

        let total_rarity_weight = parsed
            .drops
            .common_weight
            .saturating_add(parsed.drops.uncommon_weight)
            .saturating_add(parsed.drops.rare_weight)
            .saturating_add(parsed.drops.mythic_weight);
        let total_season_weight = parsed
            .drops
            .in_season_weight_multiplier
            .saturating_add(parsed.drops.off_season_weight_multiplier);
        if total_rarity_weight == 0 || total_season_weight == 0 {
            return Err(BalanceError::ZeroDropWeights);
        }

        let species_bp = multiplier_to_bp(
            "default_species_multiplier",
            parsed.growth.default_species_multiplier,
        )?;
        let season_bp = multiplier_to_bp(
            "default_season_multiplier",
            parsed.growth.default_season_multiplier,
        )?;
        let preferred_season_bp = multiplier_to_bp(
            "preferred_season_multiplier",
            parsed.seasons.preferred_season_multiplier,
        )?;
        let off_season_bp = multiplier_to_bp(
            "off_season_multiplier",
            parsed.seasons.off_season_multiplier,
        )?;
        let default_profile =
            GrowthProfile::new(parsed.growth.default_stage_thresholds, species_bp)?;

        Ok(Self {
            default_plots: parsed.garden.default_plots,
            base_gp_per_tick: parsed.growth.base_gp_per_tick,
            sun_gp_per_tick: parsed.growth.sun_gp_per_tick,
            default_profile,
            default_season_multiplier_bp: season_bp,
            dormant_after_days: parsed.dormancy.dormant_after_days,
            seasons: SeasonBalance {
                season_length_days: parsed.seasons.season_length_days,
                preferred_season_multiplier_bp: preferred_season_bp,
                off_season_multiplier_bp: off_season_bp,
            },
            drops: DropBalance {
                common_weight: parsed.drops.common_weight,
                uncommon_weight: parsed.drops.uncommon_weight,
                rare_weight: parsed.drops.rare_weight,
                mythic_weight: parsed.drops.mythic_weight,
                in_season_weight_multiplier: parsed.drops.in_season_weight_multiplier,
                off_season_weight_multiplier: parsed.drops.off_season_weight_multiplier,
                seeds_per_harvest: parsed.drops.seeds_per_harvest,
            },
        })
    }

    #[must_use]
    pub const fn dormant_after_ticks(&self) -> u64 {
        (self.dormant_after_days as u64).saturating_mul(TICKS_PER_DAY)
    }

    /// Computes exact integer per-tick GP using `GP = (base + sun + water) × species × season`
    /// with basis-point multipliers (`10_000 == 1.0x`).
    #[must_use]
    pub fn compute_tick_gp(
        &self,
        active: bool,
        water_gp: u64,
        species_multiplier_bp: u32,
        season_multiplier_bp: u32,
    ) -> u64 {
        let sun = if active { self.sun_gp_per_tick } else { 0 };
        let raw = u128::from(self.base_gp_per_tick)
            .saturating_add(u128::from(sun))
            .saturating_add(u128::from(water_gp));
        let denom = u128::from(MULTIPLIER_BP_SCALE) * u128::from(MULTIPLIER_BP_SCALE);
        let scaled = raw
            .saturating_mul(u128::from(species_multiplier_bp))
            .saturating_mul(u128::from(season_multiplier_bp))
            / denom;
        u64::try_from(scaled).unwrap_or(u64::MAX)
    }
}

impl Default for Balance {
    fn default() -> Self {
        Self::load_default()
    }
}
