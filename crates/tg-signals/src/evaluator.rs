//! Pure `WorkEvent` → [`CatchUpEvent`] evaluator enforcing consent, shell spacing,
//! diminishing returns, and per-hour / per-day water caps (PLAN §3.1, §4.1, §5 & §10 T09).

use crate::consent::is_event_consented;
use tg_core::balance::SignalBalance;
use tg_core::catch_up::CatchUpEvent;
use tg_core::clock::{Timestamp, TICK_SECS};
use tg_store::config::AppConfig;
use tg_store::save::WaterBudgetState;
use tg_store::spool::{DurationBucket, WorkEvent};

const SECS_PER_UTC_HOUR: i64 = 3_600;
const SECS_PER_UTC_DAY: i64 = 86_400;

/// Result of evaluating a batch of spool [`WorkEvent`]s up to `now`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EvaluationOutcome {
    /// Timestamped active/water inputs ready for [`tg_core::sim::Garden::catch_up_with_events`].
    /// Includes consented out-of-window (`ts < created_at` or `ts > now`) events with
    /// `active = false, water_gp = 0` so `tg-core` increments diagnostic counters.
    pub catch_up_events: Vec<CatchUpEvent>,
    /// Consented events falling inside an incomplete current tick (`new_last_tick_at < ts <= now`)
    /// that must remain in `events.jsonl` until their 60-second tick finishes.
    pub retained_spool_events: Vec<WorkEvent>,
    /// Count of spool events dropped because their signal was disabled or `ts < consented_at`.
    pub dropped_unconsented_events: u64,
    /// Total water GP granted across all applicable completed-tick events in this batch.
    pub total_water_gp_granted: u64,
}

/// Scales `raw_water_gp` by integer hyperbolic diminishing returns `K / (K + n)`
/// where `K = half_life_events.max(1)` and `n = prior_events_in_hour` (PLAN §4.1).
#[must_use]
pub fn diminish_water_gp(
    raw_water_gp: u64,
    prior_events_in_hour: u64,
    half_life_events: u64,
) -> u64 {
    let k = u128::from(half_life_events.max(1));
    let n = u128::from(prior_events_in_hour);
    let numer = u128::from(raw_water_gp).saturating_mul(k);
    let denom = k.saturating_add(n);
    u64::try_from(numer / denom).unwrap_or(u64::MAX)
}

/// Evaluates spool [`WorkEvent`]s against `config`, `signal_balance`, and `budget`.
///
/// Pure function: performs zero disk, clock, or environment I/O.
pub fn evaluate_work_events(
    events: &[WorkEvent],
    config: &AppConfig,
    signal_balance: &SignalBalance,
    garden_created_at: Timestamp,
    garden_last_tick_at: Timestamp,
    now: Timestamp,
    budget: &mut WaterBudgetState,
) -> EvaluationOutcome {
    // On backwards clock skew (`now < garden_last_tick_at`), `Garden::catch_up_with_events`
    // advances 0 ticks. Preserve consented spool events without mutating `budget`.
    if now < garden_last_tick_at {
        let mut retained_spool_events = Vec::new();
        let mut dropped_unconsented_events = 0_u64;
        for &ev in events {
            if is_event_consented(config, ev) {
                retained_spool_events.push(ev);
            } else {
                dropped_unconsented_events = dropped_unconsented_events.saturating_add(1);
            }
        }
        return EvaluationOutcome {
            catch_up_events: Vec::new(),
            retained_spool_events,
            dropped_unconsented_events,
            total_water_gp_granted: 0,
        };
    }

    let elapsed_secs = now
        .as_unix_secs()
        .saturating_sub(garden_last_tick_at.as_unix_secs());
    let total_ticks = u64::try_from(elapsed_secs / TICK_SECS).unwrap_or(0);
    let new_last_tick_at = garden_last_tick_at.add_ticks(total_ticks);

    let mut catch_up_events = Vec::new();
    let mut retained_spool_events = Vec::new();
    let mut applicable: Vec<(usize, WorkEvent)> = Vec::new();
    let mut dropped_unconsented_events = 0_u64;

    for (idx, &ev) in events.iter().enumerate() {
        if !is_event_consented(config, ev) {
            dropped_unconsented_events = dropped_unconsented_events.saturating_add(1);
            continue;
        }

        let ts = ev.timestamp();
        if ts > now || ts < garden_created_at {
            // Pass through with 0 water so `Garden::catch_up_with_events` records
            // `dropped_future_events` / `dropped_pre_creation_events` without charging caps.
            catch_up_events.push(CatchUpEvent::with_water(ts, false, 0));
        } else if total_ticks == 0 || ts > new_last_tick_at {
            // Falls inside the currently incomplete `< 60 s` tick window (or arrived while
            // `< 60 s` elapsed since `garden_last_tick_at`); retain until a tick completes.
            retained_spool_events.push(ev);
        } else {
            // `garden_created_at <= ts <= new_last_tick_at` with `total_ticks >= 1`.
            // Previously applied events were already removed by `EventSpool::compact`, so any
            // consented event still present with `ts < garden_last_tick_at` (e.g. a commit
            // discovered on a 5-minute periodic git poll after 60-second daemon ticks advanced
            // `garden_last_tick_at`) is evaluated against its true `ts` UTC hour/day water
            // budget and applied at `effective_ts = ts.max(garden_last_tick_at + 1)`.
            applicable.push((idx, ev));
        }
    }

    applicable.sort_by_key(|&(idx, ev)| (ev.timestamp(), idx));

    let min_spacing_secs = i64::try_from(signal_balance.shell_min_spacing_secs).unwrap_or(i64::MAX);
    let first_tick_floor = garden_last_tick_at.add_secs(1);
    let mut total_water_gp_granted = 0_u64;

    for (_, ev) in applicable {
        let ts = ev.timestamp();
        let effective_ts = ts.max(first_tick_floor);
        roll_budget_buckets(budget, ts);

        match ev {
            WorkEvent::Activity { active, .. } => {
                catch_up_events.push(CatchUpEvent::with_water(effective_ts, active, 0));
            }
            WorkEvent::GitCommit { .. } => {
                let granted = award_water(signal_balance.commit_water_gp, signal_balance, budget);
                total_water_gp_granted = total_water_gp_granted.saturating_add(granted);
                catch_up_events.push(CatchUpEvent::with_water(effective_ts, false, granted));
            }
            WorkEvent::Shell { dur_bucket, .. } => {
                let within_cooldown = budget.last_shell_ts.is_some_and(|prev| {
                    ts.as_unix_secs().saturating_sub(prev.as_unix_secs()) < min_spacing_secs
                });
                let granted = if within_cooldown {
                    0
                } else {
                    budget.last_shell_ts = Some(ts);
                    let raw = shell_base_water_gp(signal_balance, dur_bucket);
                    award_water(raw, signal_balance, budget)
                };
                total_water_gp_granted = total_water_gp_granted.saturating_add(granted);
                catch_up_events.push(CatchUpEvent::with_water(effective_ts, true, granted));
            }
        }
    }

    EvaluationOutcome {
        catch_up_events,
        retained_spool_events,
        dropped_unconsented_events,
        total_water_gp_granted,
    }
}

fn roll_budget_buckets(budget: &mut WaterBudgetState, ts: Timestamp) {
    let secs = ts.as_unix_secs();
    let hour = secs.div_euclid(SECS_PER_UTC_HOUR);
    let day = secs.div_euclid(SECS_PER_UTC_DAY);

    if budget.day_bucket != Some(day) {
        budget.day_bucket = Some(day);
        budget.day_water_gp = 0;
    }
    if budget.hour_bucket != Some(hour) {
        budget.hour_bucket = Some(hour);
        budget.hour_water_gp = 0;
        budget.hour_water_events = 0;
    }
}

fn award_water(
    raw_water_gp: u64,
    signal_balance: &SignalBalance,
    budget: &mut WaterBudgetState,
) -> u64 {
    let diminished = diminish_water_gp(
        raw_water_gp,
        budget.hour_water_events,
        signal_balance.diminishing_half_life_events,
    );
    let hour_headroom = signal_balance
        .hourly_water_cap_gp
        .saturating_sub(budget.hour_water_gp);
    let day_headroom = signal_balance
        .daily_water_cap_gp
        .saturating_sub(budget.day_water_gp);
    let granted = diminished.min(hour_headroom).min(day_headroom);

    budget.hour_water_events = budget.hour_water_events.saturating_add(1);
    budget.hour_water_gp = budget.hour_water_gp.saturating_add(granted);
    budget.day_water_gp = budget.day_water_gp.saturating_add(granted);
    granted
}

const fn shell_base_water_gp(signal_balance: &SignalBalance, bucket: DurationBucket) -> u64 {
    match bucket {
        DurationBucket::Instant => signal_balance.shell_instant_water_gp,
        DurationBucket::Fast => signal_balance.shell_fast_water_gp,
        DurationBucket::Medium => signal_balance.shell_medium_water_gp,
        DurationBucket::Long => signal_balance.shell_long_water_gp,
    }
}
