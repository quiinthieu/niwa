//! Step-by-step `garden.json` schema migration framework (PLAN §6 & T05).

use crate::error::StoreError;
use serde_json::{json, Value};
use std::path::Path;

/// Current `garden.json` schema version (v1).
pub const CURRENT_SAVE_SCHEMA_VERSION: u32 = 1;

/// Single-step migration upgrading a save JSON value from `source_version()` to `source_version() + 1`.
pub trait SaveMigration: Send + Sync {
    fn source_version(&self) -> u32;

    fn target_version(&self) -> u32 {
        self.source_version() + 1
    }

    fn migrate(&self, value: &mut Value) -> Result<(), StoreError>;
}

/// Baseline `v0 -> v1` sample migration (T05).
#[derive(Copy, Clone, Debug, Default)]
pub struct MigrateV0ToV1;

impl SaveMigration for MigrateV0ToV1 {
    fn source_version(&self) -> u32 {
        0
    }

    fn migrate(&self, value: &mut Value) -> Result<(), StoreError> {
        let Some(obj) = value.as_object_mut() else {
            return Err(StoreError::CorruptSave {
                path: Path::new("garden.json").to_path_buf(),
                reason: "v0 save root must be a JSON object".to_string(),
            });
        };

        obj.entry("inventory".to_string())
            .or_insert_with(|| json!({}));
        obj.entry("almanac".to_string())
            .or_insert_with(|| json!({}));
        obj.entry("diagnostics".to_string()).or_insert_with(|| {
            json!({
                "dropped_future_events": 0,
                "dropped_pre_creation_events": 0,
                "corrupt_spool_lines": 0,
                "clock_skew_events": 0
            })
        });

        Ok(())
    }
}

/// Outcome of running migrations on a parsed JSON document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigrationReport {
    pub value: Value,
    pub original_version: u32,
    pub final_version: u32,
    pub steps_applied: u32,
}

/// Ordered runner applying [`SaveMigration`] steps one version at a time.
pub struct MigrationRunner {
    target_version: u32,
    steps: Vec<Box<dyn SaveMigration>>,
}

impl Default for MigrationRunner {
    fn default() -> Self {
        Self::new(CURRENT_SAVE_SCHEMA_VERSION, vec![Box::new(MigrateV0ToV1)])
    }
}

impl MigrationRunner {
    #[must_use]
    pub fn new(target_version: u32, steps: Vec<Box<dyn SaveMigration>>) -> Self {
        Self {
            target_version,
            steps,
        }
    }

    pub fn migrate_value(
        &self,
        path: &Path,
        mut value: Value,
    ) -> Result<MigrationReport, StoreError> {
        let Some(obj) = value.as_object() else {
            return Err(StoreError::CorruptSave {
                path: path.to_path_buf(),
                reason: "save root JSON value must be an object".to_string(),
            });
        };

        let original_version = obj
            .get("schema_version")
            .and_then(Value::as_u64)
            .and_then(|raw| u32::try_from(raw).ok())
            .ok_or_else(|| StoreError::CorruptSave {
                path: path.to_path_buf(),
                reason: "missing or invalid integer 'schema_version'".to_string(),
            })?;

        if original_version > self.target_version {
            return Err(StoreError::UnsupportedFutureSchema {
                kind: "garden.json",
                found: original_version,
                max_supported: self.target_version,
            });
        }

        let mut current = original_version;
        let mut steps_applied = 0;

        while current < self.target_version {
            let step = self
                .steps
                .iter()
                .find(|candidate| candidate.source_version() == current)
                .ok_or(StoreError::MigrationGap {
                    from: current,
                    expected_max: self.target_version,
                })?;

            step.migrate(&mut value)?;
            let next = step.target_version();
            if let Some(map) = value.as_object_mut() {
                map.insert("schema_version".to_string(), Value::from(next));
            }
            current = next;
            steps_applied += 1;
        }

        Ok(MigrationReport {
            value,
            original_version,
            final_version: current,
            steps_applied,
        })
    }
}
