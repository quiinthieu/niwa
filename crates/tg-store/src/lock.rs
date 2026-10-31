//! Cross-platform advisory exclusive file lock on `garden.lock` (ADR 002, PLAN §6).

use crate::error::StoreError;
use crate::paths::NiwaPaths;
use fs4::fs_std::FileExt;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

/// Owned RAII guard holding an exclusive advisory lock on `garden.lock`.
#[derive(Debug)]
pub struct GardenLock {
    file: File,
    path: PathBuf,
}

impl GardenLock {
    /// Blocks until an exclusive lock on `paths.lock_file()` is acquired.
    pub fn acquire(paths: &NiwaPaths) -> Result<Self, StoreError> {
        let (file, path) = open_lock_file(paths)?;
        file.lock_exclusive().map_err(|source| StoreError::Io {
            path: path.clone(),
            action: "acquire exclusive lock",
            source,
        })?;
        Ok(Self { file, path })
    }

    /// Attempts a non-blocking exclusive lock on `paths.lock_file()`, returning `Ok(None)` if contended.
    pub fn try_acquire(paths: &NiwaPaths) -> Result<Option<Self>, StoreError> {
        let (file, path) = open_lock_file(paths)?;
        let acquired = file.try_lock_exclusive().map_err(|source| StoreError::Io {
            path: path.clone(),
            action: "try exclusive lock",
            source,
        })?;
        if acquired {
            Ok(Some(Self { file, path }))
        } else {
            Ok(None)
        }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for GardenLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

fn open_lock_file(paths: &NiwaPaths) -> Result<(File, PathBuf), StoreError> {
    paths.ensure_dirs().map_err(|source| StoreError::Io {
        path: paths.data_dir().to_path_buf(),
        action: "create data directory for lock file",
        source,
    })?;
    let path = paths.lock_file();
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&path)
        .map_err(|source| StoreError::Io {
            path: path.clone(),
            action: "open garden lock file",
            source,
        })?;
    Ok((file, path))
}
