//! Tunable gameplay balance parameters loaded from `data/balance.toml` (PLAN §0.9 & §4).

use crate::clock::{TICKS_PER_DAY, TICKS_PER_HOUR};
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
    #[error("catch_up.chunk_ticks must be at least 1")]
    ZeroChunkTicks,
    #[error("catch_up.offline_tail_factor must be <= 1.0")]
    TailFactorExceedsOne,
    #[error("drop table weights must include at least one positive rarity and season weight")]
    ZeroDropWeights,
    #[error("signals.diminishing_half_life_events must be at least 1")]
    ZeroDiminishingHalfLife,
    #[error("signals.hourly_water_cap_gp must be at least 1")]
    ZeroWaterCap,
    #[error("signals.daily_water_cap_gp must be >= signals.hourly_water_cap_gp")]
    DailyCapBelowHourlyCap,
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
    catch_up: RawCatchUpSection,
    #[serde(default)]
    seasons: RawSeasonsSection,
    #[serde(default)]
    drops: RawDropsSection,
    #[serde(default)]
    signals: RawSignalsSection,
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
struct RawCatchUpSection {
    offline_cap_hours: u32,
    offline_tail_factor: f64,
    chunk_ticks: u64,
}

impl Default for RawCatchUpSection {
    fn default() -> Self {
        Self {
            offline_cap_hours: 72,
            offline_tail_factor: 0.5,
            chunk_ticks: TICKS_PER_DAY,
        }
    }
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSignalsSection {
    commit_water_gp: u64,
    shell_instant_water_gp: u64,
    shell_fast_water_gp: u64,
    shell_medium_water_gp: u64,
    shell_long_water_gp: u64,
    shell_min_spacing_secs: u64,
    diminishing_half_life_events: u64,
    hourly_water_cap_gp: u64,
    daily_water_cap_gp: u64,
}

impl Default for RawSignalsSection {
    fn default() -> Self {
        let defaults = SignalBalance::default();
        Self {
            commit_water_gp: defaults.commit_water_gp,
            shell_instant_water_gp: defaults.shell_instant_water_gp,
            shell_fast_water_gp: defaults.shell_fast_water_gp,
            shell_medium_water_gp: defaults.shell_medium_water_gp,
            shell_long_water_gp: defaults.shell_long_water_gp,
            shell_min_spacing_secs: defaults.shell_min_spacing_secs,
            diminishing_half_life_events: defaults.diminishing_half_life_events,
            hourly_water_cap_gp: defaults.hourly_water_cap_gp,
            daily_water_cap_gp: defaults.daily_water_cap_gp,
        }
    }
}

/// Validated offline catch-up parameters (`offline_cap`, tail factor, and chunk size; PLAN §4.1 & T06).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatchUpBalance {
    pub offline_cap_hours: u32,
    pub offline_tail_factor_bp: u32,
    pub chunk_ticks: u64,
}

impl CatchUpBalance {
    #[must_use]
    pub const fn offline_cap_ticks(&self) -> u64 {
        (self.offline_cap_hours as u64).saturating_mul(TICKS_PER_HOUR)
    }
}

impl Default for CatchUpBalance {
    fn default() -> Self {
        Self {
            offline_cap_hours: 72,
            offline_tail_factor_bp: 5_000,
            chunk_ticks: TICKS_PER_DAY,
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

/// Validated work-signal water rewards, spacing, diminishing returns, and hourly/daily caps (PLAN §4.1 & T09).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignalBalance {
    pub commit_water_gp: u64,
    pub shell_instant_water_gp: u64,
    pub shell_fast_water_gp: u64,
    pub shell_medium_water_gp: u64,
    pub shell_long_water_gp: u64,
    pub shell_min_spacing_secs: u64,
    pub diminishing_half_life_events: u64,
    pub hourly_water_cap_gp: u64,
    pub daily_water_cap_gp: u64,
}

impl Default for SignalBalance {
    fn default() -> Self {
        Self {
            commit_water_gp: 450,
            shell_instant_water_gp: 8,
            shell_fast_water_gp: 40,
            shell_medium_water_gp: 90,
            shell_long_water_gp: 160,
            shell_min_spacing_secs: 5,
            diminishing_half_life_events: 10,
            hourly_water_cap_gp: 1_200,
            daily_water_cap_gp: 4_800,
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
    pub catch_up: CatchUpBalance,
    pub seasons: SeasonBalance,
    pub drops: DropBalance,
    pub signals: SignalBalance,
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
        if parsed.catch_up.chunk_ticks == 0 {
            return Err(BalanceError::ZeroChunkTicks);
        }
        let offline_tail_factor_bp =
            multiplier_to_bp("offline_tail_factor", parsed.catch_up.offline_tail_factor)?;
        if offline_tail_factor_bp > MULTIPLIER_BP_SCALE {
            return Err(BalanceError::TailFactorExceedsOne);
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
        if parsed.signals.diminishing_half_life_events == 0 {
            return Err(BalanceError::ZeroDiminishingHalfLife);
        }
        if parsed.signals.hourly_water_cap_gp == 0 {
            return Err(BalanceError::ZeroWaterCap);
        }
        if parsed.signals.daily_water_cap_gp < parsed.signals.hourly_water_cap_gp {
            return Err(BalanceError::DailyCapBelowHourlyCap);
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
            catch_up: CatchUpBalance {
                offline_cap_hours: parsed.catch_up.offline_cap_hours,
                offline_tail_factor_bp,
                chunk_ticks: parsed.catch_up.chunk_ticks,
            },
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
            signals: SignalBalance {
                commit_water_gp: parsed.signals.commit_water_gp,
                shell_instant_water_gp: parsed.signals.shell_instant_water_gp,
                shell_fast_water_gp: parsed.signals.shell_fast_water_gp,
                shell_medium_water_gp: parsed.signals.shell_medium_water_gp,
                shell_long_water_gp: parsed.signals.shell_long_water_gp,
                shell_min_spacing_secs: parsed.signals.shell_min_spacing_secs,
                diminishing_half_life_events: parsed.signals.diminishing_half_life_events,
                hourly_water_cap_gp: parsed.signals.hourly_water_cap_gp,
                daily_water_cap_gp: parsed.signals.daily_water_cap_gp,
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
        self.compute_tick_gp_with_base_scale(
            active,
            water_gp,
            MULTIPLIER_BP_SCALE,
            species_multiplier_bp,
            season_multiplier_bp,
        )
    }

    /// Computes exact integer per-tick GP scaling `base` by `base_scale_bp` (`10_000 == 1.0x`,
    /// or `offline_tail_factor_bp` after `offline_cap`).
    #[must_use]
    pub fn compute_tick_gp_with_base_scale(
        &self,
        active: bool,
        water_gp: u64,
        base_scale_bp: u32,
        species_multiplier_bp: u32,
        season_multiplier_bp: u32,
    ) -> u64 {
        let sun = if active { self.sun_gp_per_tick } else { 0 };
        let scale = u128::from(MULTIPLIER_BP_SCALE);
        let raw_bp = u128::from(self.base_gp_per_tick)
            .saturating_mul(u128::from(base_scale_bp))
            .saturating_add(u128::from(sun).saturating_mul(scale))
            .saturating_add(u128::from(water_gp).saturating_mul(scale));
        let denom = scale * scale * scale;
        let scaled = raw_bp
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
