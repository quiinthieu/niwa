//! Calendar and garden season resolution with Northern/Southern hemisphere support (PLAN §4.2 & T04).

use crate::clock::{Timestamp, TICKS_PER_DAY, TICK_SECS};
use serde::{Deserialize, Serialize};
use time::{Date, Month, OffsetDateTime, PrimitiveDateTime, Time};

/// The four garden seasons.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Season {
    Spring,
    Summer,
    Autumn,
    Winter,
}

impl Season {
    pub const ALL: [Self; 4] = [Self::Spring, Self::Summer, Self::Autumn, Self::Winter];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Spring => "spring",
            Self::Summer => "summer",
            Self::Autumn => "autumn",
            Self::Winter => "winter",
        }
    }

    /// Returns the opposite hemisphere's simultaneous season.
    #[must_use]
    pub const fn opposite(self) -> Self {
        match self {
            Self::Spring => Self::Autumn,
            Self::Summer => Self::Winter,
            Self::Autumn => Self::Spring,
            Self::Winter => Self::Summer,
        }
    }

    /// Returns the next season in the annual cycle.
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Spring => Self::Summer,
            Self::Summer => Self::Autumn,
            Self::Autumn => Self::Winter,
            Self::Winter => Self::Spring,
        }
    }

    /// Resolves meteorological UTC calendar season for `now` and `hemisphere` (ADR 001).
    #[must_use]
    pub fn from_calendar(now: Timestamp, hemisphere: Hemisphere) -> Self {
        let min_ts = PrimitiveDateTime::new(Date::MIN, Time::MIDNIGHT)
            .assume_utc()
            .unix_timestamp();
        let max_ts = PrimitiveDateTime::new(Date::MAX, Time::MIDNIGHT)
            .assume_utc()
            .unix_timestamp();
        let clamped_secs = now.as_unix_secs().clamp(min_ts, max_ts);
        let dt = OffsetDateTime::from_unix_timestamp(clamped_secs)
            .expect("clamped timestamp is within OffsetDateTime range");

        let north = match dt.month() {
            Month::March | Month::April | Month::May => Self::Spring,
            Month::June | Month::July | Month::August => Self::Summer,
            Month::September | Month::October | Month::November => Self::Autumn,
            Month::December | Month::January | Month::February => Self::Winter,
        };

        match hemisphere {
            Hemisphere::North => north,
            Hemisphere::South => north.opposite(),
        }
    }

    /// Resolves `garden` mode season, cycling `Spring → Summer → Autumn → Winter`
    /// every `season_length_days` elapsed since `created_at`.
    #[must_use]
    pub fn from_garden_age(created_at: Timestamp, now: Timestamp, season_length_days: u32) -> Self {
        let effective_days = u64::from(season_length_days.max(1));
        let season_secs = effective_days
            .saturating_mul(TICKS_PER_DAY)
            .saturating_mul(TICK_SECS as u64);
        let elapsed_secs =
            u64::try_from(now.as_unix_secs().saturating_sub(created_at.as_unix_secs()))
                .unwrap_or(0);
        let index = ((elapsed_secs / season_secs) % 4) as usize;
        Self::ALL[index]
    }
}

/// Real-world calendar hemisphere (PLAN §4.2 & §6).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Hemisphere {
    #[default]
    North,
    South,
}

/// Season progression mode (`calendar` vs `garden` cycling every `season_length_days`).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeasonMode {
    #[default]
    Calendar,
    Garden,
}

/// Resolves the active [`Season`] according to `mode`, `hemisphere`, and timestamps.
#[must_use]
pub fn resolve_season(
    mode: SeasonMode,
    hemisphere: Hemisphere,
    now: Timestamp,
    garden_created_at: Timestamp,
    season_length_days: u32,
) -> Season {
    match mode {
        SeasonMode::Calendar => Season::from_calendar(now, hemisphere),
        SeasonMode::Garden => Season::from_garden_age(garden_created_at, now, season_length_days),
    }
}
