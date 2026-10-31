//! Deterministic offline `catch_up(now)` engine, clock-skew guard, and "While you were away" summary (PLAN §4.1, §7 & T06).

use crate::balance::{Balance, MULTIPLIER_BP_SCALE};
use crate::clock::{Timestamp, TICKS_PER_DAY, TICK_SECS};
use crate::model::Stage;
use crate::season::{resolve_season, Hemisphere, Season, SeasonMode};
use crate::sim::Garden;
use crate::species::SpeciesCatalog;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const SECS_PER_UTC_DAY: i64 = 86_400;

/// Per-run options passed to [`Garden::catch_up`] and [`Garden::catch_up_with_events`].
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatchUpOptions {
    pub season_mode: SeasonMode,
    pub hemisphere: Hemisphere,
    pub dormant_after_days_override: Option<u32>,
}

/// Timestamped work/activity input applied at its corresponding tick window during catch-up (PLAN §4.1).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatchUpEvent {
    pub timestamp: Timestamp,
    pub active: bool,
    pub water_gp: u64,
}

impl CatchUpEvent {
    #[must_use]
    pub const fn active_at(timestamp: Timestamp) -> Self {
        Self {
            timestamp,
            active: true,
            water_gp: 0,
        }
    }

    #[must_use]
    pub const fn with_water(timestamp: Timestamp, active: bool, water_gp: u64) -> Self {
        Self {
            timestamp,
            active,
            water_gp,
        }
    }
}

/// Recorded when `now < garden.last_tick_at()` (clock moved backwards; ignored and noted per T06).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClockSkewNote {
    pub observed_now: Timestamp,
    pub last_tick_at: Timestamp,
    pub backwards_secs: i64,
}

/// Diagnostic counter deltas produced by a single `catch_up` call (PLAN §4.1 & §6).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatchUpDiagnostics {
    pub clock_skew_events: u64,
    pub dropped_future_events: u64,
    pub dropped_pre_creation_events: u64,
}

/// Net progress of one occupied plot across a `catch_up` interval.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlantProgressEntry {
    pub plot_index: usize,
    pub species_id: String,
    pub display_name: String,
    pub from_stage: Stage,
    pub to_stage: Stage,
    pub gp_gained: u64,
}

/// Cozy "While you were away" summary constructed after [`Garden::catch_up`] (PLAN §10 T06).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AwaySummary {
    pub ticks_simulated: u64,
    pub full_offline_ticks: u64,
    pub tail_offline_ticks: u64,
    pub dormant_ticks: u64,
    pub active_ticks: u64,
    pub season_before: Season,
    pub season_after: Season,
    pub entered_dormancy: bool,
    pub is_dormant: bool,
    pub clock_skew: Option<ClockSkewNote>,
    pub stage_changes: Vec<PlantProgressEntry>,
    pub headline: String,
    pub lines: Vec<String>,
}

impl AwaySummary {
    /// Returns `true` when at least one tick was simulated or clock skew was noted.
    #[must_use]
    pub const fn has_activity_to_report(&self) -> bool {
        self.ticks_simulated > 0 || self.clock_skew.is_some()
    }

    /// Formats the summary as a deterministic, cozy human-readable block.
    #[must_use]
    pub fn render_text(&self) -> String {
        let mut out = self.headline.clone();
        for line in &self.lines {
            out.push_str("\n- ");
            out.push_str(line);
        }
        out
    }
}

/// Complete outcome returned by [`Garden::catch_up`] and [`Garden::catch_up_with_events`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatchUpOutcome {
    pub ticks_simulated: u64,
    pub chunks_processed: u64,
    pub full_offline_ticks: u64,
    pub tail_offline_ticks: u64,
    pub dormant_ticks: u64,
    pub active_ticks: u64,
    pub clock_skew: Option<ClockSkewNote>,
    pub diagnostics: CatchUpDiagnostics,
    pub summary: AwaySummary,
}

#[derive(Copy, Clone)]
struct ResolvedPlotSpec {
    thresholds: [u64; 4],
    species_multiplier_bp: u32,
    season_bp_by_season: [u32; 4],
    sprout_allowed_by_season: [bool; 4],
}

#[derive(Clone)]
struct PlotBeforeSnapshot {
    species_id: String,
    display_name: String,
    stage: Stage,
    total_gp: u64,
}

impl Garden {
    /// Catches the garden up to `now` in bounded chunks with no external work events.
    pub fn catch_up(
        &mut self,
        now: Timestamp,
        options: CatchUpOptions,
        balance: &Balance,
        catalog: &SpeciesCatalog,
    ) -> CatchUpOutcome {
        self.catch_up_with_events(now, &[], options, balance, catalog)
    }

    /// Catches the garden up to `now` in bounded chunks, applying timestamped [`CatchUpEvent`]s.
    pub fn catch_up_with_events(
        &mut self,
        now: Timestamp,
        events: &[CatchUpEvent],
        options: CatchUpOptions,
        balance: &Balance,
        catalog: &SpeciesCatalog,
    ) -> CatchUpOutcome {
        let season_length_days = balance.seasons.season_length_days;
        let season_before = resolve_season(
            options.season_mode,
            options.hemisphere,
            self.last_tick_at,
            self.created_at,
            season_length_days,
        );

        if now < self.last_tick_at {
            let backwards_secs = self
                .last_tick_at
                .as_unix_secs()
                .saturating_sub(now.as_unix_secs());
            let skew = ClockSkewNote {
                observed_now: now,
                last_tick_at: self.last_tick_at,
                backwards_secs,
            };
            let diagnostics = CatchUpDiagnostics {
                clock_skew_events: 1,
                dropped_future_events: 0,
                dropped_pre_creation_events: 0,
            };
            let summary = AwaySummary {
                ticks_simulated: 0,
                full_offline_ticks: 0,
                tail_offline_ticks: 0,
                dormant_ticks: 0,
                active_ticks: 0,
                season_before,
                season_after: season_before,
                entered_dormancy: false,
                is_dormant: self.dormant,
                clock_skew: Some(skew),
                stage_changes: Vec::new(),
                headline: "While you were away: clock skew detected".to_string(),
                lines: vec![format!(
                    "System clock moved backwards by {}; kept your garden safely at its latest moment.",
                    format_duration_secs(backwards_secs)
                )],
            };
            return CatchUpOutcome {
                ticks_simulated: 0,
                chunks_processed: 0,
                full_offline_ticks: 0,
                tail_offline_ticks: 0,
                dormant_ticks: 0,
                active_ticks: 0,
                clock_skew: Some(skew),
                diagnostics,
                summary,
            };
        }

        let elapsed_secs = now
            .as_unix_secs()
            .saturating_sub(self.last_tick_at.as_unix_secs());
        let total_ticks = u64::try_from(elapsed_secs / TICK_SECS).unwrap_or(0);

        let mut diagnostics = CatchUpDiagnostics::default();
        let mut tick_bonuses: BTreeMap<u64, (bool, u64)> = BTreeMap::new();
        for &ev in events {
            if ev.timestamp > now {
                diagnostics.dropped_future_events =
                    diagnostics.dropped_future_events.saturating_add(1);
            } else if ev.timestamp < self.created_at {
                diagnostics.dropped_pre_creation_events =
                    diagnostics.dropped_pre_creation_events.saturating_add(1);
            } else if ev.timestamp > self.last_tick_at {
                let offset_secs = ev.timestamp.as_unix_secs() - self.last_tick_at.as_unix_secs();
                let tick_offset = u64::try_from((offset_secs - 1) / TICK_SECS).unwrap_or(u64::MAX);
                if tick_offset < total_ticks {
                    let entry = tick_bonuses.entry(tick_offset).or_insert((false, 0));
                    entry.0 |= ev.active;
                    entry.1 = entry.1.saturating_add(ev.water_gp);
                }
            }
        }

        if total_ticks == 0 {
            let summary = AwaySummary {
                ticks_simulated: 0,
                full_offline_ticks: 0,
                tail_offline_ticks: 0,
                dormant_ticks: 0,
                active_ticks: 0,
                season_before,
                season_after: season_before,
                entered_dormancy: false,
                is_dormant: self.dormant,
                clock_skew: None,
                stage_changes: Vec::new(),
                headline: "Welcome back".to_string(),
                lines: vec![
                    "Less than a minute has passed; your garden is right where you left it."
                        .to_string(),
                ],
            };
            return CatchUpOutcome {
                ticks_simulated: 0,
                chunks_processed: 0,
                full_offline_ticks: 0,
                tail_offline_ticks: 0,
                dormant_ticks: 0,
                active_ticks: 0,
                clock_skew: None,
                diagnostics,
                summary,
            };
        }

        let was_dormant_before = self.dormant;
        let mut before_plots: Vec<Option<PlotBeforeSnapshot>> =
            Vec::with_capacity(self.plots.len());
        let mut plot_specs: Vec<Option<ResolvedPlotSpec>> = Vec::with_capacity(self.plots.len());

        for plot in &self.plots {
            if let Some(plant) = plot.plant() {
                let (spec, display_name) = resolve_plot_spec(&plant.species_id, balance, catalog);
                before_plots.push(Some(PlotBeforeSnapshot {
                    species_id: plant.species_id.clone(),
                    display_name,
                    stage: plant.stage,
                    total_gp: plant.total_gp,
                }));
                plot_specs.push(Some(spec));
            } else {
                before_plots.push(None);
                plot_specs.push(None);
            }
        }

        let dormant_days = options
            .dormant_after_days_override
            .unwrap_or(balance.dormant_after_days);
        let dormant_after_ticks = u64::from(dormant_days).saturating_mul(TICKS_PER_DAY);
        let offline_cap_ticks = balance.catch_up.offline_cap_ticks();
        let tail_bp = balance.catch_up.offline_tail_factor_bp;
        let chunk_size = balance.catch_up.chunk_ticks.max(1);

        let mut full_offline_ticks: u64 = 0;
        let mut tail_offline_ticks: u64 = 0;
        let mut dormant_ticks: u64 = 0;
        let mut active_ticks: u64 = 0;
        let mut chunks_processed: u64 = 0;
        let mut cached_calendar_day: Option<(i64, Season)> = None;

        let mut cursor: u64 = 0;
        while cursor < total_ticks {
            let chunk_end = cursor.saturating_add(chunk_size).min(total_ticks);
            chunks_processed = chunks_processed.saturating_add(1);

            let mut offset = cursor;
            while offset < chunk_end {
                if self.inactive_ticks >= dormant_after_ticks {
                    let next_event = if tick_bonuses.is_empty() {
                        None
                    } else {
                        tick_bonuses
                            .range(offset..chunk_end)
                            .next()
                            .map(|(&idx, _)| idx)
                    };
                    let idle_until = next_event.unwrap_or(chunk_end);
                    if idle_until > offset {
                        let skip = idle_until - offset;
                        self.dormant = true;
                        self.inactive_ticks = self.inactive_ticks.saturating_add(skip);
                        self.tick_index = self.tick_index.saturating_add(skip);
                        self.last_tick_at = self.last_tick_at.add_ticks(skip);
                        dormant_ticks = dormant_ticks.saturating_add(skip);
                        offset = idle_until;
                        continue;
                    }
                }

                let (active, water_gp) = if tick_bonuses.is_empty() {
                    (false, 0)
                } else {
                    tick_bonuses.get(&offset).copied().unwrap_or((false, 0))
                };

                let current_tick_index = self.tick_index;
                let next_timestamp = self.last_tick_at.add_ticks(1);
                let pre_inactive = self.inactive_ticks;

                let paused_by_dormancy = if active {
                    self.inactive_ticks = 0;
                    self.dormant = false;
                    false
                } else if pre_inactive >= dormant_after_ticks {
                    self.dormant = true;
                    self.inactive_ticks = pre_inactive.saturating_add(1);
                    true
                } else {
                    self.inactive_ticks = pre_inactive.saturating_add(1);
                    self.dormant = self.inactive_ticks >= dormant_after_ticks;
                    false
                };

                let base_scale_bp = if active {
                    active_ticks = active_ticks.saturating_add(1);
                    MULTIPLIER_BP_SCALE
                } else if paused_by_dormancy {
                    dormant_ticks = dormant_ticks.saturating_add(1);
                    0
                } else if pre_inactive >= offline_cap_ticks {
                    tail_offline_ticks = tail_offline_ticks.saturating_add(1);
                    tail_bp
                } else {
                    full_offline_ticks = full_offline_ticks.saturating_add(1);
                    MULTIPLIER_BP_SCALE
                };

                if !paused_by_dormancy {
                    let season = resolve_season_cached(
                        options.season_mode,
                        options.hemisphere,
                        next_timestamp,
                        self.created_at,
                        season_length_days,
                        &mut cached_calendar_day,
                    );
                    let s_idx = season_index(season);

                    for (plot_index, plot) in self.plots.iter_mut().enumerate() {
                        if let (Some(plant), Some(spec)) =
                            (plot.plant_mut(), plot_specs[plot_index])
                        {
                            if plant.stage == Stage::Seed && !spec.sprout_allowed_by_season[s_idx] {
                                continue;
                            }
                            let delta_gp = balance.compute_tick_gp_with_base_scale(
                                active,
                                water_gp,
                                base_scale_bp,
                                spec.species_multiplier_bp,
                                spec.season_bp_by_season[s_idx],
                            );
                            plant.advance_gp_quiet(
                                self.seed,
                                current_tick_index,
                                plot_index,
                                delta_gp,
                                &spec.thresholds,
                                next_timestamp,
                            );
                        }
                    }
                }

                self.tick_index = current_tick_index.saturating_add(1);
                self.last_tick_at = next_timestamp;
                offset += 1;
            }

            cursor = chunk_end;
        }

        let season_after = resolve_season(
            options.season_mode,
            options.hemisphere,
            self.last_tick_at,
            self.created_at,
            season_length_days,
        );
        let entered_dormancy = !was_dormant_before && self.dormant;

        let summary = build_away_summary(
            total_ticks,
            full_offline_ticks,
            tail_offline_ticks,
            dormant_ticks,
            active_ticks,
            offline_cap_ticks,
            dormant_after_ticks,
            season_before,
            season_after,
            entered_dormancy,
            self.dormant,
            &before_plots,
            &self.plots,
        );

        CatchUpOutcome {
            ticks_simulated: total_ticks,
            chunks_processed,
            full_offline_ticks,
            tail_offline_ticks,
            dormant_ticks,
            active_ticks,
            clock_skew: None,
            diagnostics,
            summary,
        }
    }
}

fn resolve_plot_spec(
    species_id: &str,
    balance: &Balance,
    catalog: &SpeciesCatalog,
) -> (ResolvedPlotSpec, String) {
    match catalog.get(species_id) {
        Some(species) => {
            let profile = species.growth_profile();
            let mut season_bp_by_season = [0_u32; 4];
            let mut sprout_allowed_by_season = [true; 4];
            for s in Season::ALL {
                let idx = season_index(s);
                season_bp_by_season[idx] =
                    balance.seasons.multiplier_bp_for(species.prefers_season(s));
                sprout_allowed_by_season[idx] = species.can_sprout_in(s);
            }
            (
                ResolvedPlotSpec {
                    thresholds: profile.stage_thresholds,
                    species_multiplier_bp: profile.species_multiplier_bp,
                    season_bp_by_season,
                    sprout_allowed_by_season,
                },
                species.display_name.clone(),
            )
        }
        None => (
            ResolvedPlotSpec {
                thresholds: balance.default_profile.stage_thresholds,
                species_multiplier_bp: balance.default_profile.species_multiplier_bp,
                season_bp_by_season: [balance.default_season_multiplier_bp; 4],
                sprout_allowed_by_season: [true; 4],
            },
            species_id.to_string(),
        ),
    }
}

fn resolve_season_cached(
    mode: SeasonMode,
    hemisphere: Hemisphere,
    now: Timestamp,
    created_at: Timestamp,
    season_length_days: u32,
    cache: &mut Option<(i64, Season)>,
) -> Season {
    match mode {
        SeasonMode::Calendar => {
            let utc_day = now.as_unix_secs().div_euclid(SECS_PER_UTC_DAY);
            if let Some((cached_day, cached_season)) = *cache {
                if cached_day == utc_day {
                    return cached_season;
                }
            }
            let resolved = Season::from_calendar(now, hemisphere);
            *cache = Some((utc_day, resolved));
            resolved
        }
        SeasonMode::Garden => Season::from_garden_age(created_at, now, season_length_days),
    }
}

const fn season_index(season: Season) -> usize {
    match season {
        Season::Spring => 0,
        Season::Summer => 1,
        Season::Autumn => 2,
        Season::Winter => 3,
    }
}

const fn season_title(season: Season) -> &'static str {
    match season {
        Season::Spring => "Spring",
        Season::Summer => "Summer",
        Season::Autumn => "Autumn",
        Season::Winter => "Winter",
    }
}

const fn stage_title(stage: Stage) -> &'static str {
    match stage {
        Stage::Seed => "Seed",
        Stage::Sprout => "Sprout",
        Stage::Young => "Young",
        Stage::Mature => "Mature",
        Stage::Blooming => "Blooming",
    }
}

#[allow(clippy::too_many_arguments)]
fn build_away_summary(
    ticks_simulated: u64,
    full_offline_ticks: u64,
    tail_offline_ticks: u64,
    dormant_ticks: u64,
    active_ticks: u64,
    offline_cap_ticks: u64,
    dormant_after_ticks: u64,
    season_before: Season,
    season_after: Season,
    entered_dormancy: bool,
    is_dormant: bool,
    before_plots: &[Option<PlotBeforeSnapshot>],
    after_plots: &[crate::model::Plot],
) -> AwaySummary {
    let headline = format!(
        "While you were away ({})",
        format_duration_ticks(ticks_simulated)
    );
    let mut lines = Vec::new();
    let mut stage_changes = Vec::new();
    let mut planted_count: usize = 0;
    let mut quietly_grew_count: usize = 0;

    for (plot_index, (before_opt, after_plot)) in
        before_plots.iter().zip(after_plots.iter()).enumerate()
    {
        if let (Some(before), Some(after)) = (before_opt.as_ref(), after_plot.plant()) {
            planted_count += 1;
            let gp_gained = after.total_gp.saturating_sub(before.total_gp);
            if after.stage > before.stage {
                let plot_num = plot_index + 1;
                if after.stage == Stage::Blooming {
                    lines.push(format!(
                        "Plot {plot_num}: {} bloomed (from {}).",
                        before.display_name,
                        stage_title(before.stage)
                    ));
                } else {
                    lines.push(format!(
                        "Plot {plot_num}: {} grew from {} to {}.",
                        before.display_name,
                        stage_title(before.stage),
                        stage_title(after.stage)
                    ));
                }
                stage_changes.push(PlantProgressEntry {
                    plot_index,
                    species_id: before.species_id.clone(),
                    display_name: before.display_name.clone(),
                    from_stage: before.stage,
                    to_stage: after.stage,
                    gp_gained,
                });
            } else if gp_gained > 0 {
                quietly_grew_count += 1;
            }
        }
    }

    if stage_changes.is_empty() {
        if planted_count == 0 {
            lines.push(
                "No seeds are planted yet — your plots are ready whenever you are.".to_string(),
            );
        } else if quietly_grew_count > 0 {
            let noun = if quietly_grew_count == 1 {
                "plant"
            } else {
                "plants"
            };
            lines.push(format!(
                "{quietly_grew_count} {noun} grew quietly in place."
            ));
        } else {
            lines.push("Your plants rested quietly with nothing lost.".to_string());
        }
    }

    if season_before != season_after {
        lines.push(format!(
            "Season shifted from {} to {}.",
            season_title(season_before),
            season_title(season_after)
        ));
    }

    if entered_dormancy {
        lines.push(format!(
            "After {} without activity, your garden drifted into a cozy rest — nothing is lost.",
            format_duration_ticks(dormant_after_ticks)
        ));
    } else if is_dormant {
        lines.push(
            "Your garden remained in a cozy rest — activity will wake it right up.".to_string(),
        );
    } else if tail_offline_ticks > 0 {
        lines.push(format!(
            "After {} away, growth eased into a gentle resting pace.",
            format_duration_ticks(offline_cap_ticks)
        ));
    }

    AwaySummary {
        ticks_simulated,
        full_offline_ticks,
        tail_offline_ticks,
        dormant_ticks,
        active_ticks,
        season_before,
        season_after,
        entered_dormancy,
        is_dormant,
        clock_skew: None,
        stage_changes,
        headline,
        lines,
    }
}

fn format_duration_ticks(ticks: u64) -> String {
    let days = ticks / TICKS_PER_DAY;
    let rem_after_days = ticks % TICKS_PER_DAY;
    let hours = rem_after_days / 60;
    let mins = rem_after_days % 60;

    if days > 0 {
        if hours > 0 {
            format!("{days}d {hours}h")
        } else if mins > 0 {
            format!("{days}d {mins}m")
        } else {
            format!("{days}d")
        }
    } else if hours > 0 {
        if mins > 0 {
            format!("{hours}h {mins}m")
        } else {
            format!("{hours}h")
        }
    } else {
        format!("{mins}m")
    }
}

fn format_duration_secs(secs: i64) -> String {
    let clamped = secs.max(0);
    if clamped < TICK_SECS {
        return format!("{clamped}s");
    }
    let ticks = u64::try_from(clamped / TICK_SECS).unwrap_or(0);
    let rem_secs = clamped % TICK_SECS;
    if rem_secs == 0 {
        format_duration_ticks(ticks)
    } else {
        format!("{} {rem_secs}s", format_duration_ticks(ticks))
    }
}
