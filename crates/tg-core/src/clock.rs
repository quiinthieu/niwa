//! Pure time representation, `Clock` injection trait, and `FakeClock`.
//!
//! `tg-core` never reads the system clock directly; callers inject a `Clock`
//! (`SystemClock` from `tg-store` in production, `FakeClock` in tests).

use core::sync::atomic::{AtomicI64, Ordering};
use serde::{Deserialize, Serialize};

/// Duration of one garden simulation tick in seconds (PLAN §2).
pub const TICK_SECS: i64 = 60;

/// Number of 60-second simulation ticks in one 24-hour day.
pub const TICKS_PER_DAY: u64 = 24 * 60;

/// Unix timestamp in whole seconds since `1970-01-01T00:00:00Z`.
#[derive(
    Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Timestamp(pub i64);

impl Timestamp {
    #[must_use]
    pub const fn from_unix_secs(secs: i64) -> Self {
        Self(secs)
    }

    #[must_use]
    pub const fn as_unix_secs(self) -> i64 {
        self.0
    }

    #[must_use]
    pub const fn add_secs(self, delta_secs: i64) -> Self {
        Self(self.0.saturating_add(delta_secs))
    }

    #[must_use]
    pub const fn add_ticks(self, ticks: u64) -> Self {
        let delta = ticks.saturating_mul(TICK_SECS as u64);
        let capped = if delta > i64::MAX as u64 {
            i64::MAX
        } else {
            delta as i64
        };
        Self(self.0.saturating_add(capped))
    }
}

/// Pure time source injected into core garden operations.
pub trait Clock: Send + Sync {
    fn now(&self) -> Timestamp;
}

impl<C: Clock + ?Sized> Clock for &C {
    fn now(&self) -> Timestamp {
        (**self).now()
    }
}

/// Deterministic in-memory clock for unit, property, and golden tests.
#[derive(Debug)]
pub struct FakeClock {
    now_secs: AtomicI64,
}

impl FakeClock {
    #[must_use]
    pub const fn new(initial: Timestamp) -> Self {
        Self {
            now_secs: AtomicI64::new(initial.0),
        }
    }

    #[must_use]
    pub const fn from_unix_secs(secs: i64) -> Self {
        Self::new(Timestamp::from_unix_secs(secs))
    }

    pub fn set(&self, timestamp: Timestamp) {
        self.now_secs.store(timestamp.0, Ordering::SeqCst);
    }

    /// Advances the clock by `delta_secs` (negative values simulate backwards clock skew)
    /// and returns the updated timestamp.
    pub fn advance_secs(&self, delta_secs: i64) -> Timestamp {
        let prev = self.now_secs.fetch_add(delta_secs, Ordering::SeqCst);
        Timestamp(prev.wrapping_add(delta_secs))
    }

    /// Advances the clock by `ticks * TICK_SECS` and returns the updated timestamp.
    pub fn advance_ticks(&self, ticks: u64) -> Timestamp {
        let delta = i64::try_from(ticks.saturating_mul(TICK_SECS as u64)).unwrap_or(i64::MAX);
        self.advance_secs(delta)
    }
}

impl Default for FakeClock {
    fn default() -> Self {
        Self::from_unix_secs(0)
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Timestamp {
        Timestamp(self.now_secs.load(Ordering::SeqCst))
    }
}
