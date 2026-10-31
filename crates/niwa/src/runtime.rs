//! Runtime state lifecycle (`lock → load → catch_up → save → snapshot`), domain actions,
//! and structured `status --json` reporting (PLAN §3.4, §4.1, §4.2, §6 & T08).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use tg_core::balance::Balance;
use tg_core::catch_up::{AwaySummary, CatchUpOptions};
use tg_core::clock::{Clock, Timestamp, TICK_SECS};
use tg_core::drops::{DropTable, DropTableError};
use tg_core::model::{SimError, Stage};
use tg_core::product;
use tg_core::rng::rng_for_stream;
use tg_core::season::{resolve_season, Season};
use tg_core::sim::{Garden, TickInput};
use tg_core::species::{Rarity, SpeciesCatalog, SpriteSet};
use tg_render::{
    compact_glyph, detect_color_tier, render_compact_frame, render_garden, ColorEnv,
    ColorPreference, ColorTier, RenderOptions, RenderedFrame, SpriteBank, Viewport,
};
use tg_store::config::{AppConfig, ColorSetting, ConfigStore};
use tg_store::error::StoreError;
use tg_store::paths::NiwaPaths;
use tg_store::save::{AlmanacEntry, GardenSave, GardenStore, RecoveryNotice, SaveDiagnostics};
use tg_store::snapshot::{write_snapshot, Snapshot};

/// Current `niwa status --json` payload schema version.
pub const CURRENT_STATUS_SCHEMA_VERSION: u32 = 1;

/// Action-level failure returned by interactive TUI / session commands.
#[derive(Debug)]
pub enum ActionError {
    Store(StoreError),
    Sim(SimError),
    Drops(DropTableError),
    UnknownSpecies { species_id: String },
    OutOfSeeds { species_id: String },
    PlotNotBlooming { plot_index: usize, stage: Stage },
}

impl std::fmt::Display for ActionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Store(err) => write!(f, "{err}"),
            Self::Sim(err) => write!(f, "{err}"),
            Self::Drops(err) => write!(f, "{err}"),
            Self::UnknownSpecies { species_id } => {
                write!(f, "unknown species '{species_id}'")
            }
            Self::OutOfSeeds { species_id } => {
                write!(f, "no '{species_id}' seeds left in seed pouch")
            }
            Self::PlotNotBlooming { plot_index, stage } => write!(
                f,
                "plot {} is {:?} — wait until it is blooming to harvest",
                plot_index + 1,
                stage
            ),
        }
    }
}

impl std::error::Error for ActionError {}

impl From<StoreError> for ActionError {
    fn from(value: StoreError) -> Self {
        Self::Store(value)
    }
}

impl From<SimError> for ActionError {
    fn from(value: SimError) -> Self {
        Self::Sim(value)
    }
}

impl From<DropTableError> for ActionError {
    fn from(value: DropTableError) -> Self {
        Self::Drops(value)
    }
}

/// Summary of a successful [`GardenSession::plant`] action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlantOutcome {
    pub plot_index: usize,
    pub species_id: String,
    pub display_name: String,
}

/// Summary of a successful [`GardenSession::harvest`] action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HarvestOutcome {
    pub plot_index: usize,
    pub harvested_species_id: String,
    pub harvested_display_name: String,
    pub dropped_seeds: Vec<(String, String)>,
}

/// Summary of a successful [`GardenSession::compost`] action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompostOutcome {
    pub plot_index: usize,
    pub composted_species_id: String,
    pub composted_display_name: String,
    pub rarity: Rarity,
    pub returned_seed_id: String,
    pub returned_display_name: String,
}

/// Per-plant entry inside [`StatusReport`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlantStatus {
    pub species_id: String,
    pub display_name: String,
    pub rarity: Rarity,
    pub stage: Stage,
    pub stage_gp: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_stage_gp: Option<u64>,
    pub total_gp: u64,
    pub planted_at: Timestamp,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bloomed_at: Option<Timestamp>,
}

/// Per-plot entry inside [`StatusReport`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlotStatus {
    pub plot_index: usize,
    pub plant: Option<PlantStatus>,
}

/// Structured output emitted by `niwa status --json` (PLAN §10 T08).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusReport {
    pub schema_version: u32,
    pub app_version: String,
    pub season: Season,
    pub glyph: String,
    pub summary: String,
    pub dormant: bool,
    pub seed: u64,
    pub created_at: Timestamp,
    pub last_tick_at: Timestamp,
    pub tick_index: u64,
    pub inactive_ticks: u64,
    pub planted_count: usize,
    pub blooming_count: usize,
    pub plot_count: usize,
    pub plots: Vec<PlotStatus>,
    pub inventory: BTreeMap<String, u32>,
    pub almanac_discovered: usize,
    pub almanac_total: usize,
    pub almanac: BTreeMap<String, AlmanacEntry>,
    pub diagnostics: SaveDiagnostics,
}

/// Resolves the active [`ColorTier`] from `config.toml`, `--no-color`, and host terminal env vars.
#[must_use]
pub fn resolve_color_tier(config: &AppConfig, no_color_flag: bool) -> ColorTier {
    let preference = match config.color {
        ColorSetting::Auto => ColorPreference::Auto,
        ColorSetting::Truecolor => ColorPreference::Truecolor,
        ColorSetting::Ansi256 => ColorPreference::Ansi256,
        ColorSetting::Ansi16 => ColorPreference::Ansi16,
        ColorSetting::None => ColorPreference::None,
    };
    let no_color_env = std::env::var("NO_COLOR").ok();
    let colorterm = std::env::var("COLORTERM").ok();
    let term = std::env::var("TERM").ok();
    let env = ColorEnv {
        no_color_flag,
        no_color_env: no_color_env.as_deref(),
        colorterm: colorterm.as_deref(),
        term: term.as_deref(),
    };
    detect_color_tier(preference, &env)
}

/// Live garden session coordinating `tg-core`, `tg-store`, and `tg-render`.
#[derive(Clone, Debug)]
pub struct GardenSession {
    paths: NiwaPaths,
    config: AppConfig,
    balance: Balance,
    catalog: SpeciesCatalog,
    sprites: SpriteBank,
    save: GardenSave,
    garden: Garden,
    last_away: Option<AwaySummary>,
    recovery_notice: Option<RecoveryNotice>,
    action_nonce: u64,
}

impl GardenSession {
    /// Runs `lock → load (or init starter garden) → catch_up(now) → save → snapshot` (PLAN §3.4).
    pub fn open(
        paths: NiwaPaths,
        clock: &dyn Clock,
        seed_override: Option<u64>,
    ) -> Result<Self, StoreError> {
        let config = ConfigStore::new(paths.clone()).load()?;
        let balance = Balance::load_default();
        let catalog = SpeciesCatalog::load_default();
        let sprites = SpriteBank::load_default();
        let store = GardenStore::new(paths.clone());
        let now = clock.now();

        let (save, garden, last_away, recovery_notice) = {
            let _lock = store.lock()?;
            match store.load()? {
                Some(outcome) => {
                    let mut save = outcome.save;
                    let mut garden = save.to_garden();
                    let catch_up_opts = CatchUpOptions {
                        season_mode: config.season_mode,
                        hemisphere: config.hemisphere,
                        dormant_after_days_override: Some(config.dormant_after_days),
                    };
                    let catch_up_outcome = garden.catch_up(now, catch_up_opts, &balance, &catalog);
                    save.record_catch_up(&garden, &catch_up_outcome);
                    sync_almanac(&mut save, &garden, now);
                    store.save(&save)?;

                    let season = resolve_season(
                        config.season_mode,
                        config.hemisphere,
                        garden.last_tick_at(),
                        garden.created_at(),
                        balance.seasons.season_length_days,
                    );
                    let snap = build_snapshot(
                        &garden,
                        &config,
                        &catalog,
                        &sprites,
                        season,
                        now.max(garden.last_tick_at()),
                    );
                    write_snapshot(&paths, &snap)?;

                    let away = catch_up_outcome
                        .summary
                        .has_activity_to_report()
                        .then_some(catch_up_outcome.summary);
                    (save, garden, away, outcome.notice)
                }
                None => {
                    let seed = seed_override.unwrap_or_else(|| derive_initial_seed(now));
                    let garden = Garden::from_balance(seed, &balance, now);
                    let mut save = GardenSave::from_garden(&garden);
                    seed_starter_inventory(&mut save, &catalog, now);
                    sync_almanac(&mut save, &garden, now);
                    store.save(&save)?;

                    let season = resolve_season(
                        config.season_mode,
                        config.hemisphere,
                        garden.last_tick_at(),
                        garden.created_at(),
                        balance.seasons.season_length_days,
                    );
                    let snap = build_snapshot(&garden, &config, &catalog, &sprites, season, now);
                    write_snapshot(&paths, &snap)?;

                    (save, garden, None, None)
                }
            }
        };

        Ok(Self {
            paths,
            config,
            balance,
            catalog,
            sprites,
            save,
            garden,
            last_away,
            recovery_notice,
            action_nonce: 0,
        })
    }

    #[must_use]
    pub const fn paths(&self) -> &NiwaPaths {
        &self.paths
    }

    #[must_use]
    pub const fn config(&self) -> &AppConfig {
        &self.config
    }

    #[must_use]
    pub const fn balance(&self) -> &Balance {
        &self.balance
    }

    #[must_use]
    pub const fn catalog(&self) -> &SpeciesCatalog {
        &self.catalog
    }

    #[must_use]
    pub const fn sprites(&self) -> &SpriteBank {
        &self.sprites
    }

    #[must_use]
    pub const fn save(&self) -> &GardenSave {
        &self.save
    }

    #[must_use]
    pub const fn garden(&self) -> &Garden {
        &self.garden
    }

    #[must_use]
    pub const fn last_away(&self) -> Option<&AwaySummary> {
        self.last_away.as_ref()
    }

    pub fn take_last_away(&mut self) -> Option<AwaySummary> {
        self.last_away.take()
    }

    #[must_use]
    pub const fn recovery_notice(&self) -> Option<&RecoveryNotice> {
        self.recovery_notice.as_ref()
    }

    #[must_use]
    pub fn current_season(&self) -> Season {
        resolve_season(
            self.config.season_mode,
            self.config.hemisphere,
            self.garden.last_tick_at(),
            self.garden.created_at(),
            self.balance.seasons.season_length_days,
        )
    }

    /// Wakes a dormant garden when the user opens the interactive TUI (PLAN §1.1 G4 & §10 T08).
    pub fn wake_on_tui_open(&mut self, now: Timestamp) -> Result<(), StoreError> {
        if self.garden.is_dormant() || self.garden.inactive_ticks() > 0 {
            self.garden.wake();
            self.persist(now)?;
        }
        Ok(())
    }

    /// Advances any 60-second ticks elapsed while the TUI is open as active ticks (`sun` bonus;
    /// PLAN §4.1 & §10 T08: "The TUI counts as an active signal while open").
    pub fn advance_open_tui_ticks(&mut self, now: Timestamp) -> Result<usize, StoreError> {
        let elapsed_secs = now
            .as_unix_secs()
            .saturating_sub(self.garden.last_tick_at().as_unix_secs());
        if elapsed_secs < TICK_SECS {
            return Ok(0);
        }
        let ticks = usize::try_from(elapsed_secs / TICK_SECS).unwrap_or(0);
        if ticks == 0 {
            return Ok(0);
        }

        let mut effective_balance = self.balance.clone();
        effective_balance.dormant_after_days = self.config.dormant_after_days;

        for _ in 0..ticks {
            let next_ts = self.garden.last_tick_at().add_ticks(1);
            let season = resolve_season(
                self.config.season_mode,
                self.config.hemisphere,
                next_ts,
                self.garden.created_at(),
                effective_balance.seasons.season_length_days,
            );
            self.garden.tick_with_catalog(
                TickInput::active(),
                season,
                &effective_balance,
                &self.catalog,
            );
        }

        self.persist(now)?;
        Ok(ticks)
    }

    /// Plants `species_id` from `inventory` into `plot_index` (PLAN §4.2 & §10 T08).
    pub fn plant(
        &mut self,
        plot_index: usize,
        species_id: &str,
        now: Timestamp,
    ) -> Result<PlantOutcome, ActionError> {
        let Some(species) = self.catalog.get(species_id) else {
            return Err(ActionError::UnknownSpecies {
                species_id: species_id.to_string(),
            });
        };
        let display_name = species.display_name.clone();

        let available = self.save.inventory.get(species_id).copied().unwrap_or(0);
        if available == 0 {
            return Err(ActionError::OutOfSeeds {
                species_id: species_id.to_string(),
            });
        }

        self.garden.plant_seed(plot_index, species_id)?;
        if available == 1 {
            self.save.inventory.remove(species_id);
        } else {
            self.save
                .inventory
                .insert(species_id.to_string(), available - 1);
        }
        self.save
            .almanac
            .entry(species_id.to_string())
            .or_insert(AlmanacEntry {
                discovered_at: now,
                first_bloomed_at: None,
                harvest_count: 0,
            });

        self.persist(now)?;
        Ok(PlantOutcome {
            plot_index,
            species_id: species_id.to_string(),
            display_name,
        })
    }

    /// Harvests a `Blooming` plant at `plot_index`, clearing the plot, sampling weighted seed drops,
    /// and updating the almanac (PLAN §4.2 & §10 T08).
    pub fn harvest(
        &mut self,
        plot_index: usize,
        now: Timestamp,
    ) -> Result<HarvestOutcome, ActionError> {
        let Some(plot) = self.garden.plots().get(plot_index) else {
            return Err(ActionError::Sim(SimError::PlotOutOfBounds {
                plot_index,
                plot_count: self.garden.plots().len(),
            }));
        };
        let Some(existing) = plot.plant() else {
            return Err(ActionError::Sim(SimError::PlotEmpty { plot_index }));
        };
        if !existing.is_blooming() {
            return Err(ActionError::PlotNotBlooming {
                plot_index,
                stage: existing.stage,
            });
        }

        let removed = self.garden.clear_plot(plot_index)?;
        let harvested_display_name = self
            .catalog
            .get(&removed.species_id)
            .map_or_else(|| removed.species_id.clone(), |sp| sp.display_name.clone());

        let season = self.current_season();
        let stream = self.next_action_stream(plot_index, removed.bloom_seed.unwrap_or(0));
        let mut rng = rng_for_stream(self.garden.seed(), self.garden.tick_index(), stream);
        let drops = DropTable::new(self.balance.drops).sample_harvest_batch(
            &self.catalog,
            season,
            &mut rng,
        )?;

        let mut dropped_seeds = Vec::with_capacity(drops.len());
        for sp in drops {
            *self.save.inventory.entry(sp.id.clone()).or_insert(0) += 1;
            self.save
                .almanac
                .entry(sp.id.clone())
                .or_insert(AlmanacEntry {
                    discovered_at: now,
                    first_bloomed_at: None,
                    harvest_count: 0,
                });
            dropped_seeds.push((sp.id.clone(), sp.display_name.clone()));
        }

        let bloom_ts = removed.bloomed_at.unwrap_or(now);
        let entry = self
            .save
            .almanac
            .entry(removed.species_id.clone())
            .or_insert(AlmanacEntry {
                discovered_at: removed.planted_at,
                first_bloomed_at: Some(bloom_ts),
                harvest_count: 0,
            });
        if entry.first_bloomed_at.is_none() {
            entry.first_bloomed_at = Some(bloom_ts);
        }
        entry.harvest_count = entry.harvest_count.saturating_add(1);

        self.persist(now)?;
        Ok(HarvestOutcome {
            plot_index,
            harvested_species_id: removed.species_id,
            harvested_display_name,
            dropped_seeds,
        })
    }

    /// Composts the plant at `plot_index` (any stage) and awards 1 seed of the exact same rarity
    /// (PLAN §4.2 & §10 T08).
    pub fn compost(
        &mut self,
        plot_index: usize,
        now: Timestamp,
    ) -> Result<CompostOutcome, ActionError> {
        let removed = self.garden.clear_plot(plot_index)?;
        let (composted_display_name, rarity) = self.catalog.get(&removed.species_id).map_or_else(
            || (removed.species_id.clone(), Rarity::Common),
            |sp| (sp.display_name.clone(), sp.rarity),
        );

        let season = self.current_season();
        let stream = self.next_action_stream(plot_index, 0xC04D_504F_5354);
        let mut rng = rng_for_stream(self.garden.seed(), self.garden.tick_index(), stream);
        let dropped = DropTable::new(self.balance.drops).sample_same_rarity(
            &self.catalog,
            rarity,
            season,
            &mut rng,
        )?;
        let returned_seed_id = dropped.id.clone();
        let returned_display_name = dropped.display_name.clone();

        *self
            .save
            .inventory
            .entry(returned_seed_id.clone())
            .or_insert(0) += 1;
        self.save
            .almanac
            .entry(returned_seed_id.clone())
            .or_insert(AlmanacEntry {
                discovered_at: now,
                first_bloomed_at: None,
                harvest_count: 0,
            });

        self.persist(now)?;
        Ok(CompostOutcome {
            plot_index,
            composted_species_id: removed.species_id,
            composted_display_name,
            rarity,
            returned_seed_id,
            returned_display_name,
        })
    }

    /// Moves a plant from `from_index` to an empty `to_index` plot (PLAN §4.2 & §10 T08).
    pub fn move_plant(
        &mut self,
        from_index: usize,
        to_index: usize,
        now: Timestamp,
    ) -> Result<(), ActionError> {
        self.garden.move_plant(from_index, to_index)?;
        self.persist(now)?;
        Ok(())
    }

    /// Renders a single garden frame for `niwa peek` or the TUI canvas.
    #[must_use]
    pub fn render_frame(
        &self,
        viewport: Viewport,
        sprites_override: Option<SpriteSet>,
        color_tier: ColorTier,
    ) -> RenderedFrame {
        let options = RenderOptions {
            sprites: sprites_override.unwrap_or(self.config.sprites),
            color: color_tier,
            season: self.current_season(),
        };
        render_garden(
            &self.garden,
            &self.catalog,
            &self.sprites,
            viewport,
            options,
        )
    }

    /// Constructs the structured [`StatusReport`] for `niwa status [--json]`.
    #[must_use]
    pub fn status_report(&self) -> StatusReport {
        let season = self.current_season();
        let glyph = compact_glyph(&self.garden, self.config.sprites).to_string();
        let (planted_count, blooming_count, summary) =
            summarize_garden(&self.garden, self.garden.plots().len());

        let plots = self
            .garden
            .plots()
            .iter()
            .enumerate()
            .map(|(plot_index, plot)| {
                let plant = plot.plant().map(|p| {
                    let (display_name, rarity, thresholds) =
                        self.catalog.get(&p.species_id).map_or_else(
                            || {
                                (
                                    p.species_id.clone(),
                                    Rarity::Common,
                                    self.balance.default_profile.stage_thresholds,
                                )
                            },
                            |sp| (sp.display_name.clone(), sp.rarity, sp.gp_per_stage),
                        );
                    let next_stage_gp = p.stage.transition_index().map(|idx| thresholds[idx]);
                    PlantStatus {
                        species_id: p.species_id.clone(),
                        display_name,
                        rarity,
                        stage: p.stage,
                        stage_gp: p.stage_gp,
                        next_stage_gp,
                        total_gp: p.total_gp,
                        planted_at: p.planted_at,
                        bloomed_at: p.bloomed_at,
                    }
                });
                PlotStatus { plot_index, plant }
            })
            .collect();

        StatusReport {
            schema_version: CURRENT_STATUS_SCHEMA_VERSION,
            app_version: product::VERSION.to_string(),
            season,
            glyph,
            summary,
            dormant: self.garden.is_dormant(),
            seed: self.garden.seed(),
            created_at: self.garden.created_at(),
            last_tick_at: self.garden.last_tick_at(),
            tick_index: self.garden.tick_index(),
            inactive_ticks: self.garden.inactive_ticks(),
            planted_count,
            blooming_count,
            plot_count: self.garden.plots().len(),
            plots,
            inventory: self.save.inventory.clone(),
            almanac_discovered: self.save.almanac.len(),
            almanac_total: self.catalog.len(),
            almanac: self.save.almanac.clone(),
            diagnostics: self.save.diagnostics.clone(),
        }
    }

    fn next_action_stream(&mut self, plot_index: usize, salt: u64) -> u64 {
        self.action_nonce = self.action_nonce.wrapping_add(1);
        let plot_tag = (plot_index as u64).wrapping_add(1) << 32;
        0xA170_0000_0000_0000 ^ plot_tag ^ self.action_nonce ^ salt
    }

    fn persist(&mut self, now: Timestamp) -> Result<(), StoreError> {
        let store = GardenStore::new(self.paths.clone());
        let _lock = store.lock()?;
        self.save.apply_garden(&self.garden);
        sync_almanac(&mut self.save, &self.garden, now);
        store.save(&self.save)?;

        let season = self.current_season();
        let snap = build_snapshot(
            &self.garden,
            &self.config,
            &self.catalog,
            &self.sprites,
            season,
            now.max(self.garden.last_tick_at()),
        );
        write_snapshot(&self.paths, &snap)?;
        Ok(())
    }
}

/// Formats [`StatusReport`] as a cozy human-readable summary for `niwa status`.
#[must_use]
pub fn format_status_human(report: &StatusReport, catalog: &SpeciesCatalog) -> String {
    let mut out = String::new();
    let state_note = if report.dormant { " (resting)" } else { "" };
    let _ = writeln!(
        out,
        "{} v{} {} — {}{state_note}",
        product::DISPLAY_NAME,
        report.app_version,
        report.glyph,
        season_label(report.season),
    );
    let _ = writeln!(
        out,
        "Plots: {}/{} planted ({} blooming)",
        report.planted_count, report.plot_count, report.blooming_count
    );

    let total_seeds: u32 = report.inventory.values().copied().sum();
    if report.inventory.is_empty() {
        let _ = writeln!(out, "Seeds: 0 (pouch empty)");
    } else {
        let items: Vec<String> = report
            .inventory
            .iter()
            .map(|(id, count)| {
                let name = catalog
                    .get(id)
                    .map_or_else(|| id.clone(), |sp| sp.display_name.clone());
                format!("{name} x{count}")
            })
            .collect();
        let _ = writeln!(out, "Seeds: {total_seeds} ({})", items.join(", "));
    }

    let _ = write!(
        out,
        "Almanac: {}/{} species discovered",
        report.almanac_discovered, report.almanac_total
    );
    out
}

fn derive_initial_seed(now: Timestamp) -> u64 {
    let ts_bits = now.as_unix_secs() as u64;
    let pid_bits = u64::from(std::process::id()).rotate_left(23);
    ts_bits ^ pid_bits ^ 0x4E49_5741_4741_5244
}

fn seed_starter_inventory(save: &mut GardenSave, catalog: &SpeciesCatalog, now: Timestamp) {
    for species in catalog.iter() {
        if species.rarity == Rarity::Common && !species.seasonal_sprout_only {
            save.inventory.insert(species.id.clone(), 1);
            save.almanac
                .entry(species.id.clone())
                .or_insert(AlmanacEntry {
                    discovered_at: now,
                    first_bloomed_at: None,
                    harvest_count: 0,
                });
        }
    }
}

fn sync_almanac(save: &mut GardenSave, garden: &Garden, now: Timestamp) {
    for species_id in save.inventory.keys() {
        save.almanac
            .entry(species_id.clone())
            .or_insert(AlmanacEntry {
                discovered_at: now,
                first_bloomed_at: None,
                harvest_count: 0,
            });
    }
    for plot in garden.plots() {
        if let Some(plant) = plot.plant() {
            let entry = save
                .almanac
                .entry(plant.species_id.clone())
                .or_insert(AlmanacEntry {
                    discovered_at: plant.planted_at,
                    first_bloomed_at: plant.bloomed_at,
                    harvest_count: 0,
                });
            if entry.first_bloomed_at.is_none() && plant.is_blooming() {
                entry.first_bloomed_at = Some(plant.bloomed_at.unwrap_or(garden.last_tick_at()));
            }
        }
    }
}

fn summarize_garden(garden: &Garden, total_plots: usize) -> (usize, usize, String) {
    let mut planted = 0_usize;
    let mut blooming = 0_usize;
    for plot in garden.plots() {
        if let Some(plant) = plot.plant() {
            planted += 1;
            if plant.is_blooming() {
                blooming += 1;
            }
        }
    }
    let summary = if garden.is_dormant() {
        format!("{planted}/{total_plots} planted · {blooming} blooming · resting")
    } else {
        format!("{planted}/{total_plots} planted · {blooming} blooming")
    };
    (planted, blooming, summary)
}

fn build_snapshot(
    garden: &Garden,
    config: &AppConfig,
    catalog: &SpeciesCatalog,
    sprites: &SpriteBank,
    season: Season,
    updated_at: Timestamp,
) -> Snapshot {
    let glyph = compact_glyph(garden, config.sprites);
    let (planted_count, blooming_count, summary) = summarize_garden(garden, garden.plots().len());
    let compact_frame = render_compact_frame(garden, catalog, sprites, config.sprites, season);

    let mut snap = Snapshot::new(glyph, summary, season, compact_frame, updated_at);
    snap.dormant = garden.is_dormant();
    snap.planted_count = planted_count;
    snap.blooming_count = blooming_count;
    snap
}

const fn season_label(season: Season) -> &'static str {
    match season {
        Season::Spring => "Spring",
        Season::Summer => "Summer",
        Season::Autumn => "Autumn",
        Season::Winter => "Winter",
    }
}
