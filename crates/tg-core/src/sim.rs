//! Pure garden state machine, `tick()`, dormancy, and per-tick seeding.

use crate::balance::{Balance, GrowthProfile};
use crate::clock::{Clock, Timestamp};
use crate::model::{Plant, Plot, SimError, Stage};
use crate::rng::{rng_for_stream, rng_for_tick, GardenRng};
use crate::season::Season;
use crate::species::SpeciesCatalog;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Resolves a species identifier to its [`GrowthProfile`].
pub trait ProfileResolver {
    fn profile_for(&self, species_id: &str) -> GrowthProfile;
}

impl ProfileResolver for Balance {
    fn profile_for(&self, _species_id: &str) -> GrowthProfile {
        self.default_profile.clone()
    }
}

impl ProfileResolver for (&Balance, &BTreeMap<String, GrowthProfile>) {
    fn profile_for(&self, species_id: &str) -> GrowthProfile {
        self.1
            .get(species_id)
            .cloned()
            .unwrap_or_else(|| self.0.default_profile.clone())
    }
}

impl<F> ProfileResolver for F
where
    F: Fn(&str) -> GrowthProfile,
{
    fn profile_for(&self, species_id: &str) -> GrowthProfile {
        (self)(species_id)
    }
}

/// Per-tick simulation inputs (activity signal, water bonus, optional season multiplier).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct TickInput {
    pub active: bool,
    pub water_gp: u64,
    pub season_multiplier_bp: Option<u32>,
}

impl TickInput {
    #[must_use]
    pub const fn idle() -> Self {
        Self {
            active: false,
            water_gp: 0,
            season_multiplier_bp: None,
        }
    }

    #[must_use]
    pub const fn active() -> Self {
        Self {
            active: true,
            water_gp: 0,
            season_multiplier_bp: None,
        }
    }

    #[must_use]
    pub const fn with_water(active: bool, water_gp: u64) -> Self {
        Self {
            active,
            water_gp,
            season_multiplier_bp: None,
        }
    }

    #[must_use]
    pub const fn with_season_multiplier_bp(mut self, bp: u32) -> Self {
        self.season_multiplier_bp = Some(bp);
        self
    }
}

/// Record of a single stage transition observed during a tick.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StageTransition {
    pub plot_index: usize,
    pub from: Stage,
    pub to: Stage,
}

/// Summary returned by a single [`Garden::tick`] step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TickOutcome {
    pub tick_index: u64,
    pub timestamp: Timestamp,
    pub dormant: bool,
    pub tick_roll: u64,
    pub transitions: Vec<StageTransition>,
}

/// Pure, deterministic garden state (PLAN §3.1 & §4.1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Garden {
    pub(crate) seed: u64,
    pub(crate) plots: Vec<Plot>,
    pub(crate) created_at: Timestamp,
    pub(crate) last_tick_at: Timestamp,
    pub(crate) tick_index: u64,
    pub(crate) inactive_ticks: u64,
    pub(crate) dormant: bool,
}

impl Garden {
    #[must_use]
    pub fn new(seed: u64, plot_count: usize, now: Timestamp) -> Self {
        Self {
            seed,
            plots: vec![Plot::empty(); plot_count],
            created_at: now,
            last_tick_at: now,
            tick_index: 0,
            inactive_ticks: 0,
            dormant: false,
        }
    }

    #[must_use]
    pub fn from_balance(seed: u64, balance: &Balance, now: Timestamp) -> Self {
        Self::new(seed, balance.default_plots, now)
    }

    #[must_use]
    pub fn from_clock<C: Clock>(seed: u64, balance: &Balance, clock: &C) -> Self {
        Self::from_balance(seed, balance, clock.now())
    }

    /// Reconstructs a [`Garden`] from persisted state fields without performing I/O.
    #[must_use]
    pub fn from_parts(
        seed: u64,
        plots: Vec<Plot>,
        created_at: Timestamp,
        last_tick_at: Timestamp,
        tick_index: u64,
        inactive_ticks: u64,
        dormant: bool,
    ) -> Self {
        Self {
            seed,
            plots,
            created_at,
            last_tick_at,
            tick_index,
            inactive_ticks,
            dormant,
        }
    }

    #[must_use]
    pub const fn seed(&self) -> u64 {
        self.seed
    }

    #[must_use]
    pub fn plots(&self) -> &[Plot] {
        &self.plots
    }

    #[must_use]
    pub const fn created_at(&self) -> Timestamp {
        self.created_at
    }

    #[must_use]
    pub const fn last_tick_at(&self) -> Timestamp {
        self.last_tick_at
    }

    #[must_use]
    pub const fn tick_index(&self) -> u64 {
        self.tick_index
    }

    #[must_use]
    pub const fn inactive_ticks(&self) -> u64 {
        self.inactive_ticks
    }

    #[must_use]
    pub const fn is_dormant(&self) -> bool {
        self.dormant
    }

    fn ensure_plot_in_bounds(&self, plot_index: usize) -> Result<(), SimError> {
        let plot_count = self.plots.len();
        if plot_index >= plot_count {
            return Err(SimError::PlotOutOfBounds {
                plot_index,
                plot_count,
            });
        }
        Ok(())
    }

    pub fn plant_seed(
        &mut self,
        plot_index: usize,
        species_id: impl Into<String>,
    ) -> Result<&Plant, SimError> {
        self.ensure_plot_in_bounds(plot_index)?;
        if !self.plots[plot_index].is_empty() {
            return Err(SimError::PlotOccupied { plot_index });
        }
        let plant = Plant::new(species_id, self.last_tick_at, self.tick_index);
        self.plots[plot_index].set_plant(plant);
        Ok(self.plots[plot_index]
            .plant()
            .expect("just inserted plant into plot"))
    }

    pub fn clear_plot(&mut self, plot_index: usize) -> Result<Plant, SimError> {
        self.ensure_plot_in_bounds(plot_index)?;
        self.plots[plot_index]
            .take_plant()
            .ok_or(SimError::PlotEmpty { plot_index })
    }

    pub fn move_plant(&mut self, from_index: usize, to_index: usize) -> Result<(), SimError> {
        self.ensure_plot_in_bounds(from_index)?;
        self.ensure_plot_in_bounds(to_index)?;
        if self.plots[from_index].is_empty() {
            return Err(SimError::PlotEmpty {
                plot_index: from_index,
            });
        }
        if from_index == to_index {
            return Ok(());
        }
        if !self.plots[to_index].is_empty() {
            return Err(SimError::PlotOccupied {
                plot_index: to_index,
            });
        }
        let plant = self.plots[from_index]
            .take_plant()
            .expect("verified non-empty source plot");
        self.plots[to_index].set_plant(plant);
        Ok(())
    }

    /// Wakes a dormant garden upon user return without advancing a tick.
    pub fn wake(&mut self) {
        self.inactive_ticks = 0;
        self.dormant = false;
    }

    /// Deterministic garden-wide PRNG for the next tick (`(garden_seed, tick_index)`, stream 0).
    #[must_use]
    pub fn rng_for_current_tick(&self) -> GardenRng {
        rng_for_tick(self.seed, self.tick_index)
    }

    /// Deterministic plot-scoped PRNG for the next tick (`(garden_seed, tick_index, plot_index + 1)`).
    #[must_use]
    pub fn rng_for_plot_tick(&self, plot_index: usize) -> GardenRng {
        let stream = (plot_index as u64).saturating_add(1);
        rng_for_stream(self.seed, self.tick_index, stream)
    }

    pub fn tick(&mut self, input: TickInput, balance: &Balance) -> TickOutcome {
        self.tick_with_profiles(input, balance, balance)
    }

    pub fn tick_with_profiles<R: ProfileResolver + ?Sized>(
        &mut self,
        input: TickInput,
        balance: &Balance,
        resolver: &R,
    ) -> TickOutcome {
        let current_tick_index = self.tick_index;
        let tick_roll = self.rng_for_current_tick().next_u64();
        let next_timestamp = self.last_tick_at.add_ticks(1);

        let dormant_after_ticks = balance.dormant_after_ticks();
        let paused_by_dormancy = if input.active {
            self.inactive_ticks = 0;
            self.dormant = false;
            false
        } else if self.inactive_ticks >= dormant_after_ticks {
            self.dormant = true;
            self.inactive_ticks = self.inactive_ticks.saturating_add(1);
            true
        } else {
            self.inactive_ticks = self.inactive_ticks.saturating_add(1);
            self.dormant = self.inactive_ticks >= dormant_after_ticks;
            false
        };

        let mut transitions = Vec::new();
        if !paused_by_dormancy {
            let season_bp = input
                .season_multiplier_bp
                .unwrap_or(balance.default_season_multiplier_bp);

            for (plot_index, plot) in self.plots.iter_mut().enumerate() {
                if let Some(plant) = plot.plant_mut() {
                    let profile = resolver.profile_for(&plant.species_id);
                    let delta_gp = balance.compute_tick_gp(
                        input.active,
                        input.water_gp,
                        profile.species_multiplier_bp,
                        season_bp,
                    );
                    let stream = (plot_index as u64).saturating_add(1);
                    let mut plot_rng = rng_for_stream(self.seed, current_tick_index, stream);
                    let plot_transitions = plant.advance_gp(
                        plot_index,
                        delta_gp,
                        &profile.stage_thresholds,
                        next_timestamp,
                        &mut plot_rng,
                    );
                    transitions.extend(plot_transitions);
                }
            }
        }

        self.tick_index = current_tick_index.saturating_add(1);
        self.last_tick_at = next_timestamp;

        TickOutcome {
            tick_index: current_tick_index,
            timestamp: next_timestamp,
            dormant: self.dormant,
            tick_roll,
            transitions,
        }
    }

    pub fn tick_n(&mut self, count: u64, input: TickInput, balance: &Balance) {
        self.tick_n_with_profiles(count, input, balance, balance);
    }

    pub fn tick_n_with_profiles<R: ProfileResolver + ?Sized>(
        &mut self,
        count: u64,
        input: TickInput,
        balance: &Balance,
        resolver: &R,
    ) {
        for _ in 0..count {
            self.tick_with_profiles(input, balance, resolver);
        }
    }

    pub fn simulate_ticks(&mut self, inputs: &[TickInput], balance: &Balance) -> Vec<TickOutcome> {
        self.simulate_ticks_with_profiles(inputs, balance, balance)
    }

    pub fn simulate_ticks_with_profiles<R: ProfileResolver + ?Sized>(
        &mut self,
        inputs: &[TickInput],
        balance: &Balance,
        resolver: &R,
    ) -> Vec<TickOutcome> {
        let mut outcomes = Vec::with_capacity(inputs.len());
        for &input in inputs {
            outcomes.push(self.tick_with_profiles(input, balance, resolver));
        }
        outcomes
    }

    pub fn tick_with_catalog(
        &mut self,
        input: TickInput,
        season: Season,
        balance: &Balance,
        catalog: &SpeciesCatalog,
    ) -> TickOutcome {
        let current_tick_index = self.tick_index;
        let tick_roll = self.rng_for_current_tick().next_u64();
        let next_timestamp = self.last_tick_at.add_ticks(1);

        let dormant_after_ticks = balance.dormant_after_ticks();
        let paused_by_dormancy = if input.active {
            self.inactive_ticks = 0;
            self.dormant = false;
            false
        } else if self.inactive_ticks >= dormant_after_ticks {
            self.dormant = true;
            self.inactive_ticks = self.inactive_ticks.saturating_add(1);
            true
        } else {
            self.inactive_ticks = self.inactive_ticks.saturating_add(1);
            self.dormant = self.inactive_ticks >= dormant_after_ticks;
            false
        };

        let mut transitions = Vec::new();
        if !paused_by_dormancy {
            for (plot_index, plot) in self.plots.iter_mut().enumerate() {
                if let Some(plant) = plot.plant_mut() {
                    let (profile, season_bp, can_grow) = match catalog.get(&plant.species_id) {
                        Some(species) => {
                            let sprout_allowed =
                                plant.stage != Stage::Seed || species.can_sprout_in(season);
                            let s_bp = input.season_multiplier_bp.unwrap_or_else(|| {
                                balance
                                    .seasons
                                    .multiplier_bp_for(species.prefers_season(season))
                            });
                            (species.growth_profile(), s_bp, sprout_allowed)
                        }
                        None => {
                            let s_bp = input
                                .season_multiplier_bp
                                .unwrap_or(balance.default_season_multiplier_bp);
                            (balance.default_profile.clone(), s_bp, true)
                        }
                    };

                    if !can_grow {
                        continue;
                    }

                    let delta_gp = balance.compute_tick_gp(
                        input.active,
                        input.water_gp,
                        profile.species_multiplier_bp,
                        season_bp,
                    );
                    let stream = (plot_index as u64).saturating_add(1);
                    let mut plot_rng = rng_for_stream(self.seed, current_tick_index, stream);
                    let plot_transitions = plant.advance_gp(
                        plot_index,
                        delta_gp,
                        &profile.stage_thresholds,
                        next_timestamp,
                        &mut plot_rng,
                    );
                    transitions.extend(plot_transitions);
                }
            }
        }

        self.tick_index = current_tick_index.saturating_add(1);
        self.last_tick_at = next_timestamp;

        TickOutcome {
            tick_index: current_tick_index,
            timestamp: next_timestamp,
            dormant: self.dormant,
            tick_roll,
            transitions,
        }
    }
}
