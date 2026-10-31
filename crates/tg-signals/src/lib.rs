//! tg-signals: Opt-in work signal consent disclosures, collection gate, and diminishing-returns
//! water-cap evaluator (PLAN §3.1, §4.1, §5 & §10 T09).

pub mod consent;
pub mod evaluator;

pub use tg_store::config::{ParseSignalKindError, SignalConsent, SignalKind, SignalsConfig};
pub use tg_store::save::WaterBudgetState;
pub use tg_store::spool::{
    DurationBucket, EventSpool, PurgeOutcome, ShellExitClass, SpoolCounts, SpoolReadOutcome,
    WorkEvent,
};
