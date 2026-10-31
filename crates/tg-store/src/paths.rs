//! File and directory layout inside Niwa's data and configuration roots.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Resolved filesystem layout for Niwa state, config, and lock files (PLAN §6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NiwaPaths {
    data_dir: PathBuf,
    config_dir: PathBuf,
}

impl NiwaPaths {
    #[must_use]
    pub fn new(data_dir: impl Into<PathBuf>, config_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
            config_dir: config_dir.into(),
        }
    }

    /// Resolves all data, state, and config files inside a single `NIWA_HOME` root.
    #[must_use]
    pub fn from_niwa_home(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            data_dir: root.clone(),
            config_dir: root,
        }
    }

    #[must_use]
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    #[must_use]
    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    #[must_use]
    pub fn garden_file(&self) -> PathBuf {
        self.data_dir.join("garden.json")
    }

    #[must_use]
    pub fn garden_backup_file(&self) -> PathBuf {
        self.data_dir.join("garden.json.bak")
    }

    #[must_use]
    pub fn events_file(&self) -> PathBuf {
        self.data_dir.join("events.jsonl")
    }

    #[must_use]
    pub fn snapshot_file(&self) -> PathBuf {
        self.data_dir.join("snapshot.json")
    }

    #[must_use]
    pub fn lock_file(&self) -> PathBuf {
        self.data_dir.join("garden.lock")
    }

    #[must_use]
    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    /// Creates `data_dir` and `config_dir` if they do not yet exist.
    pub fn ensure_dirs(&self) -> io::Result<()> {
        fs::create_dir_all(&self.data_dir)?;
        if self.config_dir != self.data_dir {
            fs::create_dir_all(&self.config_dir)?;
        }
        Ok(())
    }
}
