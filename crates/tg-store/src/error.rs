//! Error types returned by `tg-store` persistence, locking, migration, and config operations.

use std::io;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("I/O error while attempting to {action} '{path}': {source}")]
    Io {
        path: PathBuf,
        action: &'static str,
        #[source]
        source: io::Error,
    },

    #[error("JSON error in '{path}': {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("failed to parse TOML config at '{path}': {source}")]
    TomlParse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },

    #[error("failed to serialize TOML config: {source}")]
    TomlSerialize {
        #[source]
        source: toml::ser::Error,
    },

    #[error("could not resolve user home directory; set NIWA_HOME or HOME")]
    MissingHomeDirectory,

    #[error("simulated write abort for '{path}' at fault point '{point}'")]
    AbortedWrite { path: PathBuf, point: &'static str },

    #[error("corrupt save data in '{path}': {reason}")]
    CorruptSave { path: PathBuf, reason: String },

    #[error(
        "corrupt garden save at '{primary_path}' ({primary_reason}) and backup '{backup_path}' could not be restored ({backup_reason}); refusing to silently reset"
    )]
    CorruptSaveUnrecoverable {
        primary_path: PathBuf,
        primary_reason: String,
        backup_path: PathBuf,
        backup_reason: String,
    },

    #[error(
        "unsupported {kind} schema_version {found} (this version supports up to {max_supported})"
    )]
    UnsupportedFutureSchema {
        kind: &'static str,
        found: u32,
        max_supported: u32,
    },

    #[error("missing migration step from schema_version {from} toward {expected_max}")]
    MigrationGap { from: u32, expected_max: u32 },

    #[error("unknown config key '{key}'")]
    InvalidConfigKey { key: String },

    #[error("invalid value '{value}' for config key '{key}': {reason}")]
    InvalidConfigValue {
        key: String,
        value: String,
        reason: String,
    },

    #[error("malformed Niwa shell hook markers in '{path}': {reason}")]
    MalformedShellHookBlock { path: PathBuf, reason: String },
}
