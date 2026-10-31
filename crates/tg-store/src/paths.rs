//! File and directory layout inside Niwa's data and configuration roots.

use crate::error::StoreError;
use directories::BaseDirs;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use tg_core::product::{CLI_BIN, DISPLAY_NAME};

/// Target operating system path convention (PLAN §6).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PlatformKind {
    /// macOS: `~/Library/Application Support/Niwa/` for both state and config.
    MacOs,
    /// Linux / WSL (XDG): `$XDG_STATE_HOME/niwa/` for state, `$XDG_CONFIG_HOME/niwa/` for config.
    Linux,
}

impl PlatformKind {
    #[must_use]
    pub const fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else {
            Self::Linux
        }
    }
}

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

    /// Resolves paths from the current host environment (`NIWA_HOME`, XDG vars, and `directories`).
    pub fn resolve() -> Result<Self, StoreError> {
        let base_dirs = BaseDirs::new();
        let fallback_home = base_dirs.as_ref().map(BaseDirs::home_dir);
        Self::resolve_from_env(
            PlatformKind::current(),
            |key| std::env::var_os(key),
            fallback_home,
        )
    }

    /// Pure, deterministic path resolver accepting an injected platform, env lookup, and fallback home.
    pub fn resolve_from_env<F>(
        platform: PlatformKind,
        mut get_env: F,
        fallback_home: Option<&Path>,
    ) -> Result<Self, StoreError>
    where
        F: FnMut(&str) -> Option<OsString>,
    {
        if let Some(niwa_home) = non_empty_os(get_env("NIWA_HOME")) {
            return Ok(Self::from_niwa_home(PathBuf::from(niwa_home)));
        }

        let home = non_empty_os(get_env("HOME"))
            .map(PathBuf::from)
            .or_else(|| fallback_home.map(Path::to_path_buf))
            .ok_or(StoreError::MissingHomeDirectory)?;

        match platform {
            PlatformKind::MacOs => {
                let app_support = home
                    .join("Library")
                    .join("Application Support")
                    .join(DISPLAY_NAME);
                Ok(Self::new(app_support.clone(), app_support))
            }
            PlatformKind::Linux => {
                let state_root = non_empty_os(get_env("XDG_STATE_HOME"))
                    .map(PathBuf::from)
                    .unwrap_or_else(|| home.join(".local").join("state"));
                let config_root = non_empty_os(get_env("XDG_CONFIG_HOME"))
                    .map(PathBuf::from)
                    .unwrap_or_else(|| home.join(".config"));
                Ok(Self::new(
                    state_root.join(CLI_BIN),
                    config_root.join(CLI_BIN),
                ))
            }
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

fn non_empty_os(val: Option<OsString>) -> Option<OsString> {
    val.filter(|v| !v.is_empty())
}
