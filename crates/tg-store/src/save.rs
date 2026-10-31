//! Versioned `garden.json` v1 model, atomic persistence, and `.bak` recovery (PLAN §6 & T05).

use crate::atomic::{atomic_write_bytes, atomic_write_bytes_with_fault, FaultPoint};
use crate::error::StoreError;
use crate::lock::GardenLock;
use crate::migrate::{MigrationRunner, CURRENT_SAVE_SCHEMA_VERSION};
use crate::paths::NiwaPaths;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use tg_core::clock::Timestamp;
use tg_core::model::Plot;
use tg_core::sim::Garden;

/// Per-species discovery and bloom history stored in `garden.json` (PLAN §4.2 & §6).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlmanacEntry {
    pub discovered_at: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_bloomed_at: Option<Timestamp>,
    #[serde(default)]
    pub harvest_count: u64,
}

/// Diagnostic counters recorded in `garden.json` (PLAN §4.1, §5 & §6).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveDiagnostics {
    #[serde(default)]
    pub dropped_future_events: u64,
    #[serde(default)]
    pub dropped_pre_creation_events: u64,
    #[serde(default)]
    pub corrupt_spool_lines: u64,
    #[serde(default)]
    pub clock_skew_events: u64,
}

/// Versioned `garden.json` v1 persistence payload (PLAN §6).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GardenSave {
    pub schema_version: u32,
    pub seed: u64,
    pub plots: Vec<Plot>,
    #[serde(default)]
    pub inventory: BTreeMap<String, u32>,
    #[serde(default)]
    pub almanac: BTreeMap<String, AlmanacEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<Timestamp>,
    pub last_tick_at: Timestamp,
    #[serde(default)]
    pub tick_index: u64,
    #[serde(default)]
    pub inactive_ticks: u64,
    #[serde(default)]
    pub dormant: bool,
    #[serde(default)]
    pub diagnostics: SaveDiagnostics,
}

impl GardenSave {
    #[must_use]
    pub fn from_garden(garden: &Garden) -> Self {
        Self {
            schema_version: CURRENT_SAVE_SCHEMA_VERSION,
            seed: garden.seed(),
            plots: garden.plots().to_vec(),
            inventory: BTreeMap::new(),
            almanac: BTreeMap::new(),
            created_at: Some(garden.created_at()),
            last_tick_at: garden.last_tick_at(),
            tick_index: garden.tick_index(),
            inactive_ticks: garden.inactive_ticks(),
            dormant: garden.is_dormant(),
            diagnostics: SaveDiagnostics::default(),
        }
    }

    pub fn apply_garden(&mut self, garden: &Garden) {
        self.schema_version = CURRENT_SAVE_SCHEMA_VERSION;
        self.seed = garden.seed();
        self.plots = garden.plots().to_vec();
        self.created_at = Some(garden.created_at());
        self.last_tick_at = garden.last_tick_at();
        self.tick_index = garden.tick_index();
        self.inactive_ticks = garden.inactive_ticks();
        self.dormant = garden.is_dormant();
    }

    #[must_use]
    pub fn to_garden(&self) -> Garden {
        Garden::from_parts(
            self.seed,
            self.plots.clone(),
            self.created_at.unwrap_or(self.last_tick_at),
            self.last_tick_at,
            self.tick_index,
            self.inactive_ticks,
            self.dormant,
        )
    }
}

/// User-visible recovery notice emitted when `garden.json` had to be restored from `garden.json.bak`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecoveryNotice {
    RestoredFromBackup {
        primary_path: PathBuf,
        backup_path: PathBuf,
        reason: String,
    },
}

impl fmt::Display for RecoveryNotice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RestoredFromBackup {
                primary_path,
                backup_path,
                reason,
            } => write!(
                f,
                "Restored garden save '{}' from backup '{}' after detecting corruption ({reason}).",
                primary_path.display(),
                backup_path.display()
            ),
        }
    }
}

/// Loaded `garden.json` state paired with any recovery notice or migration metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadOutcome {
    pub save: GardenSave,
    pub notice: Option<RecoveryNotice>,
    pub migrated_from: Option<u32>,
}

/// Persistent store managing `garden.json`, `garden.json.bak`, and `garden.lock`.
#[derive(Clone, Debug)]
pub struct GardenStore {
    paths: NiwaPaths,
}

impl GardenStore {
    #[must_use]
    pub fn new(paths: NiwaPaths) -> Self {
        Self { paths }
    }

    #[must_use]
    pub fn paths(&self) -> &NiwaPaths {
        &self.paths
    }

    pub fn lock(&self) -> Result<GardenLock, StoreError> {
        GardenLock::acquire(&self.paths)
    }

    pub fn with_lock<T, F>(&self, f: F) -> Result<T, StoreError>
    where
        F: FnOnce(&Self) -> Result<T, StoreError>,
    {
        let _guard = self.lock()?;
        f(self)
    }

    pub fn load(&self) -> Result<Option<LoadOutcome>, StoreError> {
        let primary = self.paths.garden_file();
        let backup = self.paths.garden_backup_file();
        let runner = MigrationRunner::default();

        let primary_reason = match fs::read(&primary) {
            Ok(bytes) => match parse_and_migrate_save(&primary, &bytes, &runner) {
                Ok((save, migrated_from)) => {
                    return Ok(Some(LoadOutcome {
                        save,
                        notice: None,
                        migrated_from,
                    }));
                }
                Err(
                    err @ (StoreError::UnsupportedFutureSchema { .. }
                    | StoreError::MigrationGap { .. }),
                ) => {
                    return Err(err);
                }
                Err(corrupt_err) => corrupt_err.to_string(),
            },
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                if !backup.exists() {
                    return Ok(None);
                }
                "primary garden.json was missing while garden.json.bak existed".to_string()
            }
            Err(source) => {
                return Err(StoreError::Io {
                    path: primary,
                    action: "read garden save",
                    source,
                });
            }
        };

        let backup_bytes = match fs::read(&backup) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                return Err(StoreError::CorruptSaveUnrecoverable {
                    primary_path: primary,
                    primary_reason,
                    backup_path: backup,
                    backup_reason: "backup file does not exist".to_string(),
                });
            }
            Err(source) => {
                return Err(StoreError::CorruptSaveUnrecoverable {
                    primary_path: primary,
                    primary_reason,
                    backup_path: backup,
                    backup_reason: format!("failed to read backup file: {source}"),
                });
            }
        };

        match parse_and_migrate_save(&backup, &backup_bytes, &runner) {
            Ok((backup_save, migrated_from)) => {
                let healed_bytes = serialize_save_bytes(&primary, &backup_save)?;
                atomic_write_bytes(&primary, None, &healed_bytes, None)?;

                Ok(Some(LoadOutcome {
                    save: backup_save,
                    notice: Some(RecoveryNotice::RestoredFromBackup {
                        primary_path: primary,
                        backup_path: backup,
                        reason: primary_reason,
                    }),
                    migrated_from,
                }))
            }
            Err(backup_err) => Err(StoreError::CorruptSaveUnrecoverable {
                primary_path: primary,
                primary_reason,
                backup_path: backup,
                backup_reason: backup_err.to_string(),
            }),
        }
    }

    pub fn save(&self, save: &GardenSave) -> Result<(), StoreError> {
        self.save_with_fault(save, None)
    }

    pub fn save_with_fault(
        &self,
        save: &GardenSave,
        fault: Option<FaultPoint>,
    ) -> Result<(), StoreError> {
        let primary = self.paths.garden_file();
        let backup = self.paths.garden_backup_file();
        let bytes = serialize_save_bytes(&primary, save)?;
        let runner = MigrationRunner::default();
        let validate_existing =
            |existing: &[u8]| parse_and_migrate_save(&primary, existing, &runner).is_ok();

        atomic_write_bytes_with_fault(
            &primary,
            Some(&backup),
            &bytes,
            Some(&validate_existing),
            fault,
        )
    }
}

fn parse_and_migrate_save(
    path: &Path,
    bytes: &[u8],
    runner: &MigrationRunner,
) -> Result<(GardenSave, Option<u32>), StoreError> {
    let raw_value: Value =
        serde_json::from_slice(bytes).map_err(|err| StoreError::CorruptSave {
            path: path.to_path_buf(),
            reason: format!("invalid JSON: {err}"),
        })?;
    let report = runner.migrate_value(path, raw_value)?;
    let save: GardenSave =
        serde_json::from_value(report.value).map_err(|err| StoreError::CorruptSave {
            path: path.to_path_buf(),
            reason: format!("invalid garden save payload: {err}"),
        })?;
    let migrated_from = (report.steps_applied > 0).then_some(report.original_version);
    Ok((save, migrated_from))
}

fn serialize_save_bytes(path: &Path, save: &GardenSave) -> Result<Vec<u8>, StoreError> {
    let mut normalized = save.clone();
    normalized.schema_version = CURRENT_SAVE_SCHEMA_VERSION;
    let mut bytes = serde_json::to_vec_pretty(&normalized).map_err(|source| StoreError::Json {
        path: path.to_path_buf(),
        source,
    })?;
    bytes.push(b'\n');
    Ok(bytes)
}
