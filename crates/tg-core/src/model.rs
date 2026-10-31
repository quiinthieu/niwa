//! Core pure domain types: `Stage`, `Plant`, `Plot`, and `SimError`.

use crate::clock::Timestamp;
use crate::rng::GardenRng;
use crate::sim::StageTransition;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Growth lifecycle stages of a plant (`Seed → Sprout → Young → Mature → Blooming`).
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Seed,
    Sprout,
    Young,
    Mature,
    Blooming,
}

impl Stage {
    pub const ALL: [Self; 5] = [
        Self::Seed,
        Self::Sprout,
        Self::Young,
        Self::Mature,
        Self::Blooming,
    ];

    #[must_use]
    pub const fn next(self) -> Option<Self> {
        match self {
            Self::Seed => Some(Self::Sprout),
            Self::Sprout => Some(Self::Young),
            Self::Young => Some(Self::Mature),
            Self::Mature => Some(Self::Blooming),
            Self::Blooming => None,
        }
    }

    #[must_use]
    pub const fn is_blooming(self) -> bool {
        matches!(self, Self::Blooming)
    }

    #[must_use]
    pub const fn transition_index(self) -> Option<usize> {
        match self {
            Self::Seed => Some(0),
            Self::Sprout => Some(1),
            Self::Young => Some(2),
            Self::Mature => Some(3),
            Self::Blooming => None,
        }
    }
}

/// A living plant occupying a garden [`Plot`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plant {
    pub species_id: String,
    pub stage: Stage,
    pub stage_gp: u64,
    pub total_gp: u64,
    pub planted_at: Timestamp,
    pub planted_tick: u64,
    pub bloomed_at: Option<Timestamp>,
    pub bloom_seed: Option<u64>,
}

impl Plant {
    #[must_use]
    pub fn new(species_id: impl Into<String>, planted_at: Timestamp, planted_tick: u64) -> Self {
        Self {
            species_id: species_id.into(),
            stage: Stage::Seed,
            stage_gp: 0,
            total_gp: 0,
            planted_at,
            planted_tick,
            bloomed_at: None,
            bloom_seed: None,
        }
    }

    #[must_use]
    pub const fn is_blooming(&self) -> bool {
        self.stage.is_blooming()
    }

    pub(crate) fn advance_gp(
        &mut self,
        plot_index: usize,
        delta_gp: u64,
        thresholds: &[u64; 4],
        tick_timestamp: Timestamp,
        plot_rng: &mut GardenRng,
    ) -> Vec<StageTransition> {
        self.total_gp = self.total_gp.saturating_add(delta_gp);
        self.stage_gp = self.stage_gp.saturating_add(delta_gp);

        let mut transitions = Vec::new();
        while let (Some(idx), Some(next_stage)) = (self.stage.transition_index(), self.stage.next())
        {
            let threshold = thresholds[idx];
            if self.stage_gp < threshold {
                break;
            }
            self.stage_gp -= threshold;
            let from = self.stage;
            self.stage = next_stage;
            if next_stage == Stage::Blooming && self.bloomed_at.is_none() {
                self.bloomed_at = Some(tick_timestamp);
                self.bloom_seed = Some(plot_rng.next_u64());
            }
            transitions.push(StageTransition {
                plot_index,
                from,
                to: next_stage,
            });
        }
        transitions
    }
}

/// One cell in the garden grid, either empty or holding a [`Plant`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plot {
    plant: Option<Plant>,
}

impl Plot {
    #[must_use]
    pub const fn empty() -> Self {
        Self { plant: None }
    }

    #[must_use]
    pub const fn with_plant(plant: Plant) -> Self {
        Self { plant: Some(plant) }
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.plant.is_none()
    }

    #[must_use]
    pub const fn plant(&self) -> Option<&Plant> {
        self.plant.as_ref()
    }

    pub fn plant_mut(&mut self) -> Option<&mut Plant> {
        self.plant.as_mut()
    }

    pub(crate) fn set_plant(&mut self, plant: Plant) {
        self.plant = Some(plant);
    }

    pub fn take_plant(&mut self) -> Option<Plant> {
        self.plant.take()
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum SimError {
    #[error("plot index {plot_index} out of bounds for garden of {plot_count} plots")]
    PlotOutOfBounds {
        plot_index: usize,
        plot_count: usize,
    },
    #[error("plot {plot_index} already contains a plant")]
    PlotOccupied { plot_index: usize },
    #[error("plot {plot_index} is empty")]
    PlotEmpty { plot_index: usize },
}
