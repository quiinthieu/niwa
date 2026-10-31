//! System wall-clock implementation of [`tg_core::clock::Clock`].

use std::time::{SystemTime, UNIX_EPOCH};
use tg_core::clock::{Clock, Timestamp};

/// Production clock backed by [`SystemTime::now`].
#[derive(Copy, Clone, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(dur) => Timestamp::from_unix_secs(i64::try_from(dur.as_secs()).unwrap_or(i64::MAX)),
            Err(err) => {
                let secs = i64::try_from(err.duration().as_secs()).unwrap_or(i64::MAX);
                Timestamp::from_unix_secs(secs.saturating_neg())
            }
        }
    }
}
