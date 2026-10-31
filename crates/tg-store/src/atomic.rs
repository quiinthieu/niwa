//! Crash-safe atomic file writes (`temp -> fsync -> optional .bak -> rename`).

use crate::error::StoreError;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Deterministic crash-simulation fault points for testing aborted writes (T05 AC 1).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FaultPoint {
    /// Abort mid-write after flushing partial bytes to the sibling temp file.
    AfterTempPartialWrite,
    /// Abort after fsyncing the sibling temp file (and updating `.bak`), right before `rename`.
    BeforeRename,
}

pub type BackupValidator<'a> = &'a dyn Fn(&[u8]) -> bool;

struct TempFileGuard {
    path: PathBuf,
    armed: bool,
}

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// Atomically writes `bytes` to `target`, optionally rotating a valid existing `target` to `backup`.
pub fn atomic_write_bytes(
    target: &Path,
    backup: Option<&Path>,
    bytes: &[u8],
    validate_for_backup: Option<BackupValidator<'_>>,
) -> Result<(), StoreError> {
    atomic_write_bytes_with_fault(target, backup, bytes, validate_for_backup, None)
}

/// Atomically writes `bytes` to `target` with an optional injected [`FaultPoint`].
pub fn atomic_write_bytes_with_fault(
    target: &Path,
    backup: Option<&Path>,
    bytes: &[u8],
    validate_for_backup: Option<BackupValidator<'_>>,
    fault: Option<FaultPoint>,
) -> Result<(), StoreError> {
    let (mut tmp_file, tmp_path) = create_sibling_temp(target)?;
    let mut guard = TempFileGuard {
        path: tmp_path.clone(),
        armed: true,
    };

    if fault == Some(FaultPoint::AfterTempPartialWrite) {
        let half = bytes.len() / 2;
        tmp_file
            .write_all(&bytes[..half])
            .map_err(|source| StoreError::Io {
                path: tmp_path.clone(),
                action: "write partial temporary file",
                source,
            })?;
        let _ = tmp_file.flush();
        drop(tmp_file);
        // Leave partial sibling temp file on disk to simulate an abrupt process crash / SIGKILL.
        guard.armed = false;
        return Err(StoreError::AbortedWrite {
            path: target.to_path_buf(),
            point: "after_temp_partial_write",
        });
    }

    tmp_file.write_all(bytes).map_err(|source| StoreError::Io {
        path: tmp_path.clone(),
        action: "write temporary file",
        source,
    })?;
    tmp_file.sync_all().map_err(|source| StoreError::Io {
        path: tmp_path.clone(),
        action: "fsync temporary file",
        source,
    })?;
    drop(tmp_file);

    if let Some(backup_path) = backup {
        match fs::read(target) {
            Ok(existing_bytes) => {
                let should_rotate =
                    validate_for_backup.is_none_or(|validate| validate(&existing_bytes));
                if should_rotate {
                    write_single_atomic(backup_path, &existing_bytes)?;
                }
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(StoreError::Io {
                    path: target.to_path_buf(),
                    action: "read existing target before backup rotation",
                    source,
                });
            }
        }
    }

    if fault == Some(FaultPoint::BeforeRename) {
        guard.armed = false;
        return Err(StoreError::AbortedWrite {
            path: target.to_path_buf(),
            point: "before_rename",
        });
    }

    fs::rename(&tmp_path, target).map_err(|source| StoreError::Io {
        path: target.to_path_buf(),
        action: "atomically rename temporary file",
        source,
    })?;
    guard.armed = false;
    sync_parent_dir(target);

    Ok(())
}

fn write_single_atomic(target: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    let (mut tmp_file, tmp_path) = create_sibling_temp(target)?;
    let mut guard = TempFileGuard {
        path: tmp_path.clone(),
        armed: true,
    };

    tmp_file.write_all(bytes).map_err(|source| StoreError::Io {
        path: tmp_path.clone(),
        action: "write temporary backup file",
        source,
    })?;
    tmp_file.sync_all().map_err(|source| StoreError::Io {
        path: tmp_path.clone(),
        action: "fsync temporary backup file",
        source,
    })?;
    drop(tmp_file);

    fs::rename(&tmp_path, target).map_err(|source| StoreError::Io {
        path: target.to_path_buf(),
        action: "atomically rename backup file",
        source,
    })?;
    guard.armed = false;
    sync_parent_dir(target);
    Ok(())
}

fn create_sibling_temp(target: &Path) -> Result<(File, PathBuf), StoreError> {
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|source| StoreError::Io {
        path: parent.to_path_buf(),
        action: "create parent directory",
        source,
    })?;

    let base_name = target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("niwa");
    let pid = std::process::id();

    for _ in 0..64 {
        let seq = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let candidate = parent.join(format!(".{base_name}.tmp.{pid}.{seq}"));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => return Ok((file, candidate)),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(source) => {
                return Err(StoreError::Io {
                    path: candidate,
                    action: "create sibling temporary file",
                    source,
                });
            }
        }
    }

    Err(StoreError::Io {
        path: target.to_path_buf(),
        action: "allocate unique sibling temporary file",
        source: io::Error::new(
            io::ErrorKind::AlreadyExists,
            "exhausted temporary file name attempts",
        ),
    })
}

fn sync_parent_dir(target: &Path) {
    #[cfg(unix)]
    if let Some(parent) = target.parent() {
        if let Ok(dir) = File::open(parent) {
            let _ = dir.sync_all();
        }
    }
    #[cfg(not(unix))]
    let _ = target;
}
