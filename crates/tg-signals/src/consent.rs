//! Shared opt-in signal consent disclosures (`data/consent.toml`) and collection gate
//! (PLAN §0.5, §5, §10 T09 & T15).

use serde::Deserialize;
use std::fmt::Write as _;
use tg_store::config::{AppConfig, SignalKind};
use tg_store::error::StoreError;
use tg_store::spool::{EventSpool, WorkEvent};
use thiserror::Error;

/// Embedded shared `data/consent.toml` strings used by CLI and Settings UIs (PLAN §10 T09 & T15).
pub const DEFAULT_CONSENT_TOML: &str = include_str!("../../../data/consent.toml");

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ConsentError {
    #[error("failed to parse consent TOML: {0}")]
    ParseToml(String),
    #[error("consent disclosure field '{field}' for '{signal}' must not be empty")]
    EmptyField {
        signal: &'static str,
        field: &'static str,
    },
}

/// Disclosure describing Niwa's always-on wall-clock baseline (`time`).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BaselineDisclosure {
    pub display_name: String,
    pub summary: String,
    pub source: String,
    pub stores: Vec<String>,
    pub never_stores: Vec<String>,
}

/// Disclosure shown before opting in to a work signal (`activity`, `git`, or `shell`).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignalDisclosure {
    pub display_name: String,
    pub summary: String,
    pub source: String,
    pub stores: Vec<String>,
    pub never_stores: Vec<String>,
    pub prompt: String,
}

impl SignalDisclosure {
    /// Renders the multi-line consent disclosure shown by `niwa signals enable <name>` (PLAN §5).
    #[must_use]
    pub fn render_disclosure_text(&self, signal: SignalKind) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "Signal: {} ({})", signal.as_str(), self.display_name);
        let _ = writeln!(out, "  {}", self.summary);
        let _ = writeln!(out, "  Source: {}", self.source);
        let _ = writeln!(out, "\nWhat Niwa will store locally:");
        for item in &self.stores {
            let _ = writeln!(out, "  - {item}");
        }
        let _ = writeln!(out, "\nWhat Niwa NEVER stores or transmits:");
        for item in &self.never_stores {
            let _ = writeln!(out, "  - {item}");
        }
        let _ = write!(out, "\n{} [y/N]: ", self.prompt);
        out
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConsentFile {
    baseline: RawBaselineSection,
    signals: RawSignalsSection,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBaselineSection {
    time: BaselineDisclosure,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSignalsSection {
    activity: SignalDisclosure,
    git: SignalDisclosure,
    shell: SignalDisclosure,
}

/// Validated catalog of shared signal disclosures loaded from `data/consent.toml`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsentCatalog {
    time: BaselineDisclosure,
    activity: SignalDisclosure,
    git: SignalDisclosure,
    shell: SignalDisclosure,
}

impl ConsentCatalog {
    #[must_use]
    pub fn load_default() -> Self {
        Self::from_toml_str(DEFAULT_CONSENT_TOML)
            .expect("embedded data/consent.toml must parse and validate")
    }

    pub fn from_toml_str(raw: &str) -> Result<Self, ConsentError> {
        let parsed: RawConsentFile =
            toml::from_str(raw).map_err(|err| ConsentError::ParseToml(err.to_string()))?;

        validate_baseline("time", &parsed.baseline.time)?;
        validate_signal("activity", &parsed.signals.activity)?;
        validate_signal("git", &parsed.signals.git)?;
        validate_signal("shell", &parsed.signals.shell)?;

        Ok(Self {
            time: parsed.baseline.time,
            activity: parsed.signals.activity,
            git: parsed.signals.git,
            shell: parsed.signals.shell,
        })
    }

    #[must_use]
    pub const fn baseline(&self) -> &BaselineDisclosure {
        &self.time
    }

    #[must_use]
    pub const fn signal(&self, kind: SignalKind) -> &SignalDisclosure {
        match kind {
            SignalKind::Activity => &self.activity,
            SignalKind::Git => &self.git,
            SignalKind::Shell => &self.shell,
        }
    }
}

impl Default for ConsentCatalog {
    fn default() -> Self {
        Self::load_default()
    }
}

fn validate_baseline(signal: &'static str, d: &BaselineDisclosure) -> Result<(), ConsentError> {
    if d.display_name.trim().is_empty() {
        return Err(ConsentError::EmptyField {
            signal,
            field: "display_name",
        });
    }
    if d.summary.trim().is_empty() {
        return Err(ConsentError::EmptyField {
            signal,
            field: "summary",
        });
    }
    if d.source.trim().is_empty() {
        return Err(ConsentError::EmptyField {
            signal,
            field: "source",
        });
    }
    if d.stores.is_empty() || d.stores.iter().any(|s| s.trim().is_empty()) {
        return Err(ConsentError::EmptyField {
            signal,
            field: "stores",
        });
    }
    if d.never_stores.is_empty() || d.never_stores.iter().any(|s| s.trim().is_empty()) {
        return Err(ConsentError::EmptyField {
            signal,
            field: "never_stores",
        });
    }
    Ok(())
}

fn validate_signal(signal: &'static str, d: &SignalDisclosure) -> Result<(), ConsentError> {
    if d.display_name.trim().is_empty() {
        return Err(ConsentError::EmptyField {
            signal,
            field: "display_name",
        });
    }
    if d.summary.trim().is_empty() {
        return Err(ConsentError::EmptyField {
            signal,
            field: "summary",
        });
    }
    if d.source.trim().is_empty() {
        return Err(ConsentError::EmptyField {
            signal,
            field: "source",
        });
    }
    if d.stores.is_empty() || d.stores.iter().any(|s| s.trim().is_empty()) {
        return Err(ConsentError::EmptyField {
            signal,
            field: "stores",
        });
    }
    if d.never_stores.is_empty() || d.never_stores.iter().any(|s| s.trim().is_empty()) {
        return Err(ConsentError::EmptyField {
            signal,
            field: "never_stores",
        });
    }
    if d.prompt.trim().is_empty() {
        return Err(ConsentError::EmptyField {
            signal,
            field: "prompt",
        });
    }
    Ok(())
}

/// Returns `true` only when `signal` is explicitly enabled AND has a recorded `consented_at` timestamp.
#[must_use]
pub const fn is_signal_enabled(config: &AppConfig, signal: SignalKind) -> bool {
    let consent = config.signals.consent(signal);
    consent.enabled && consent.consented_at.is_some()
}

/// Returns `true` only when `event`'s signal is enabled AND `event.timestamp() >= consented_at`.
#[must_use]
pub fn is_event_consented(config: &AppConfig, event: WorkEvent) -> bool {
    let consent = config.signals.consent(event.signal_kind());
    if !consent.enabled {
        return false;
    }
    consent
        .consented_at
        .is_some_and(|consented_at| event.timestamp() >= consented_at)
}

/// Collection gate delegating persistence to [`EventSpool`] only when `event` has active consent.
///
/// Guarantees nothing is ever written to `events.jsonl` before `niwa signals enable <name>`
/// (PLAN §0.5, §0.6 & §10 T09 AC).
pub fn record_if_consented(
    config: &AppConfig,
    spool: &EventSpool,
    event: WorkEvent,
) -> Result<bool, StoreError> {
    if !is_event_consented(config, event) {
        return Ok(false);
    }
    spool.append(event)?;
    Ok(true)
}
