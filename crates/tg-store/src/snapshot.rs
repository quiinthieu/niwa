//! Versioned `snapshot.json` writer and reader shared by `niwa prompt` and the macOS menu bar (PLAN §6, §9 & T05).

use crate::atomic::atomic_write_bytes;
use crate::error::StoreError;
use crate::paths::NiwaPaths;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::io;
use tg_core::clock::Timestamp;
use tg_core::season::Season;

/// Current `snapshot.json` schema version (v1).
pub const CURRENT_SNAPSHOT_SCHEMA_VERSION: u32 = 1;

/// Compact versioned snapshot written after state updates and read by prompt/menu-bar surfaces.
///
/// Unknown JSON fields are intentionally ignored during deserialization for forward compatibility.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub schema_version: u32,
    pub glyph: String,
    pub summary: String,
    pub season: Season,
    pub compact_frame: String,
    pub updated_at: Timestamp,
    #[serde(default)]
    pub dormant: bool,
    #[serde(default)]
    pub planted_count: usize,
    #[serde(default)]
    pub blooming_count: usize,
}

impl Snapshot {
    #[must_use]
    pub fn new(
        glyph: impl Into<String>,
        summary: impl Into<String>,
        season: Season,
        compact_frame: impl Into<String>,
        updated_at: Timestamp,
    ) -> Self {
        Self {
            schema_version: CURRENT_SNAPSHOT_SCHEMA_VERSION,
            glyph: glyph.into(),
            summary: summary.into(),
            season,
            compact_frame: compact_frame.into(),
            updated_at,
            dormant: false,
            planted_count: 0,
            blooming_count: 0,
        }
    }
}

/// Atomically writes `snapshot` to `paths.snapshot_file()`.
pub fn write_snapshot(paths: &NiwaPaths, snapshot: &Snapshot) -> Result<(), StoreError> {
    let target = paths.snapshot_file();
    let mut normalized = snapshot.clone();
    normalized.schema_version = CURRENT_SNAPSHOT_SCHEMA_VERSION;
    let mut bytes = serde_json::to_vec_pretty(&normalized).map_err(|source| StoreError::Json {
        path: target.clone(),
        source,
    })?;
    bytes.push(b'\n');
    atomic_write_bytes(&target, None, &bytes, None)
}

/// Reads `paths.snapshot_file()`, returning `Ok(None)` when no snapshot has been written yet.
pub fn read_snapshot(paths: &NiwaPaths) -> Result<Option<Snapshot>, StoreError> {
    let target = paths.snapshot_file();
    let bytes = match fs::read(&target) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(StoreError::Io {
                path: target,
                action: "read snapshot.json",
                source,
            });
        }
    };

    let value: Value = serde_json::from_slice(&bytes).map_err(|source| StoreError::Json {
        path: target.clone(),
        source,
    })?;

    let version = value
        .get("schema_version")
        .and_then(Value::as_u64)
        .and_then(|v| u32::try_from(v).ok())
        .ok_or_else(|| StoreError::CorruptSave {
            path: target.clone(),
            reason: "missing or invalid integer 'schema_version' in snapshot.json".to_string(),
        })?;

    if version == 0 {
        return Err(StoreError::CorruptSave {
            path: target,
            reason: "snapshot.json schema_version must be >= 1".to_string(),
        });
    }
    if version > CURRENT_SNAPSHOT_SCHEMA_VERSION {
        return Err(StoreError::UnsupportedFutureSchema {
            kind: "snapshot.json",
            found: version,
            max_supported: CURRENT_SNAPSHOT_SCHEMA_VERSION,
        });
    }

    let snapshot: Snapshot = serde_json::from_value(value).map_err(|source| StoreError::Json {
        path: target,
        source,
    })?;
    Ok(Some(snapshot))
}
