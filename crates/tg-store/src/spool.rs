//! Privacy-safe `WorkEvent` wire schema and `events.jsonl` append/read/compact/purge spool
//! (PLAN §0.4, §0.6, §3.1, §5 & §10 T09).

use crate::atomic::atomic_write_bytes;
use crate::config::SignalKind;
use crate::error::StoreError;
use crate::lock::GardenLock;
use crate::paths::NiwaPaths;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use tg_core::clock::Timestamp;

/// Coarse exit-code classification for completed shell commands (`ok` vs `err`; PLAN §5).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShellExitClass {
    Ok,
    Err,
}

impl ShellExitClass {
    #[must_use]
    pub const fn from_exit_code(code: i32) -> Self {
        if code == 0 {
            Self::Ok
        } else {
            Self::Err
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Err => "err",
        }
    }
}

/// Coarse wall-time duration bucket for completed shell commands (PLAN §5).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DurationBucket {
    /// Under 1 second (e.g. `true`, `pwd`, `cd`).
    Instant,
    /// At least 1 second and under 10 seconds.
    Fast,
    /// At least 10 seconds and under 60 seconds.
    Medium,
    /// 60 seconds or longer.
    Long,
}

impl DurationBucket {
    #[must_use]
    pub fn from_duration_secs(secs: f64) -> Self {
        if !secs.is_finite() || secs < 1.0 {
            Self::Instant
        } else if secs < 10.0 {
            Self::Fast
        } else if secs < 60.0 {
            Self::Medium
        } else {
            Self::Long
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Instant => "instant",
            Self::Fast => "fast",
            Self::Medium => "medium",
            Self::Long => "long",
        }
    }
}

/// Privacy-safe opt-in work event stored in `events.jsonl` (PLAN §0.4, §5 & T09).
///
/// Structurally `Copy` with zero `String` or `PathBuf` fields so command text, paths,
/// repository names, commit messages, and window titles can never be represented.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum WorkEvent {
    Activity {
        ts: Timestamp,
        active: bool,
    },
    GitCommit {
        ts: Timestamp,
    },
    Shell {
        ts: Timestamp,
        status: ShellExitClass,
        dur_bucket: DurationBucket,
    },
}

impl WorkEvent {
    #[must_use]
    pub const fn timestamp(self) -> Timestamp {
        match self {
            Self::Activity { ts, .. } | Self::GitCommit { ts } | Self::Shell { ts, .. } => ts,
        }
    }

    #[must_use]
    pub const fn signal_kind(self) -> SignalKind {
        match self {
            Self::Activity { .. } => SignalKind::Activity,
            Self::GitCommit { .. } => SignalKind::Git,
            Self::Shell { .. } => SignalKind::Shell,
        }
    }
}

impl Serialize for WorkEvent {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match *self {
            Self::Activity { ts, active } => {
                let mut state = serializer.serialize_struct("WorkEvent", 3)?;
                state.serialize_field("ts", &ts)?;
                state.serialize_field("kind", "activity")?;
                state.serialize_field("active", &active)?;
                state.end()
            }
            Self::GitCommit { ts } => {
                let mut state = serializer.serialize_struct("WorkEvent", 2)?;
                state.serialize_field("ts", &ts)?;
                state.serialize_field("kind", "git_commit")?;
                state.end()
            }
            Self::Shell {
                ts,
                status,
                dur_bucket,
            } => {
                let mut state = serializer.serialize_struct("WorkEvent", 4)?;
                state.serialize_field("ts", &ts)?;
                state.serialize_field("kind", "shell")?;
                state.serialize_field("status", &status)?;
                state.serialize_field("dur_bucket", &dur_bucket)?;
                state.end()
            }
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum WireWorkEvent {
    Activity {
        ts: Timestamp,
        active: bool,
    },
    GitCommit {
        ts: Timestamp,
    },
    Shell {
        ts: Timestamp,
        status: ShellExitClass,
        dur_bucket: DurationBucket,
    },
}

impl<'de> Deserialize<'de> for WorkEvent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = WireWorkEvent::deserialize(deserializer)?;
        Ok(match wire {
            WireWorkEvent::Activity { ts, active } => Self::Activity { ts, active },
            WireWorkEvent::GitCommit { ts } => Self::GitCommit { ts },
            WireWorkEvent::Shell {
                ts,
                status,
                dur_bucket,
            } => Self::Shell {
                ts,
                status,
                dur_bucket,
            },
        })
    }
}

/// Result of reading `events.jsonl` while skipping corrupt lines (PLAN §5).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpoolReadOutcome {
    pub events: Vec<WorkEvent>,
    pub corrupt_lines: u64,
    pub bytes_read: usize,
}

/// Per-signal queued event counts in `events.jsonl` for `niwa signals list`.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct SpoolCounts {
    pub activity: usize,
    pub git: usize,
    pub shell: usize,
    pub corrupt_lines: u64,
}

impl SpoolCounts {
    #[must_use]
    pub const fn for_signal(self, signal: SignalKind) -> usize {
        match signal {
            SignalKind::Activity => self.activity,
            SignalKind::Git => self.git,
            SignalKind::Shell => self.shell,
        }
    }

    #[must_use]
    pub const fn total_valid(self) -> usize {
        self.activity
            .saturating_add(self.git)
            .saturating_add(self.shell)
    }
}

/// Summary of events removed by [`EventSpool::purge_signal`].
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct PurgeOutcome {
    pub removed_events: usize,
    pub remaining_events: usize,
}

/// Sole reader/writer for `events.jsonl` inside `tg-store` (PLAN §0.6, §3.1 & §5).
#[derive(Clone, Debug)]
pub struct EventSpool {
    paths: NiwaPaths,
}

impl EventSpool {
    #[must_use]
    pub fn new(paths: NiwaPaths) -> Self {
        Self { paths }
    }

    #[must_use]
    pub fn paths(&self) -> &NiwaPaths {
        &self.paths
    }

    /// Appends one JSONL event line with `O_APPEND` and no lock wait (PLAN §5 & §7).
    pub fn append(&self, event: WorkEvent) -> Result<(), StoreError> {
        let path = self.paths.events_file();
        let mut line = serde_json::to_vec(&event).map_err(|source| StoreError::Json {
            path: path.clone(),
            source,
        })?;
        line.push(b'\n');

        let mut file = match OpenOptions::new().create(true).append(true).open(&path) {
            Ok(file) => file,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                self.paths.ensure_dirs().map_err(|source| StoreError::Io {
                    path: self.paths.data_dir().to_path_buf(),
                    action: "create data directory for events.jsonl",
                    source,
                })?;
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)
                    .map_err(|source| StoreError::Io {
                        path: path.clone(),
                        action: "open events.jsonl for append",
                        source,
                    })?
            }
            Err(source) => {
                return Err(StoreError::Io {
                    path: path.clone(),
                    action: "open events.jsonl for append",
                    source,
                });
            }
        };
        file.write_all(&line).map_err(|source| StoreError::Io {
            path,
            action: "append line to events.jsonl",
            source,
        })
    }

    /// Reads `events.jsonl`, skipping and counting any corrupt or invalid lines (PLAN §5).
    pub fn read(&self) -> Result<SpoolReadOutcome, StoreError> {
        let path = self.paths.events_file();
        let bytes = read_spool_bytes(&path)?;
        Ok(parse_spool_bytes(&bytes))
    }

    /// Counts queued valid events per signal plus corrupt lines currently in `events.jsonl`.
    pub fn count_by_signal(&self) -> Result<SpoolCounts, StoreError> {
        let read = self.read()?;
        let mut counts = SpoolCounts {
            corrupt_lines: read.corrupt_lines,
            ..SpoolCounts::default()
        };
        for ev in read.events {
            match ev.signal_kind() {
                SignalKind::Activity => counts.activity += 1,
                SignalKind::Git => counts.git += 1,
                SignalKind::Shell => counts.shell += 1,
            }
        }
        Ok(counts)
    }

    /// Compacts the first `bytes_read` bytes of `events.jsonl` down to `retained`,
    /// preserving any new bytes concurrently appended after `bytes_read`.
    ///
    /// Callers running catch-up must hold [`GardenLock`].
    pub fn compact(&self, bytes_read: usize, retained: &[WorkEvent]) -> Result<(), StoreError> {
        let path = self.paths.events_file();
        let current = read_spool_bytes(&path)?;
        let tail = if current.len() > bytes_read {
            &current[bytes_read..]
        } else {
            &[][..]
        };

        if retained.is_empty() && tail.is_empty() {
            remove_spool_if_present(&path)?;
            return Ok(());
        }

        let mut out = serialize_events_jsonl(&path, retained)?;
        out.extend_from_slice(tail);
        atomic_write_bytes(&path, None, &out, None)
    }

    /// Deletes all queued events belonging to `signal` under [`GardenLock`], removing
    /// `events.jsonl` completely when no valid events remain (`niwa signals disable --purge`).
    pub fn purge_signal(&self, signal: SignalKind) -> Result<PurgeOutcome, StoreError> {
        let _lock = GardenLock::acquire(&self.paths)?;
        let path = self.paths.events_file();
        let bytes = read_spool_bytes(&path)?;
        if bytes.is_empty() {
            remove_spool_if_present(&path)?;
            return Ok(PurgeOutcome::default());
        }

        let parsed = parse_spool_bytes(&bytes);
        let mut removed_events = 0_usize;
        let mut retained = Vec::with_capacity(parsed.events.len());
        for ev in parsed.events {
            if ev.signal_kind() == signal {
                removed_events += 1;
            } else {
                retained.push(ev);
            }
        }

        if retained.is_empty() {
            remove_spool_if_present(&path)?;
        } else {
            let out = serialize_events_jsonl(&path, &retained)?;
            atomic_write_bytes(&path, None, &out, None)?;
        }

        Ok(PurgeOutcome {
            removed_events,
            remaining_events: retained.len(),
        })
    }
}

fn read_spool_bytes(path: &Path) -> Result<Vec<u8>, StoreError> {
    match fs::read(path) {
        Ok(bytes) => Ok(bytes),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(source) => Err(StoreError::Io {
            path: path.to_path_buf(),
            action: "read events.jsonl",
            source,
        }),
    }
}

fn remove_spool_if_present(path: &Path) -> Result<(), StoreError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(StoreError::Io {
            path: path.to_path_buf(),
            action: "remove empty events.jsonl",
            source,
        }),
    }
}

fn parse_spool_bytes(bytes: &[u8]) -> SpoolReadOutcome {
    let mut events = Vec::new();
    let mut corrupt_lines = 0_u64;

    for raw_line in bytes.split(|&b| b == b'\n') {
        let trimmed = trim_ascii_whitespace(raw_line);
        if trimmed.is_empty() {
            continue;
        }
        match serde_json::from_slice::<WorkEvent>(trimmed) {
            Ok(event) => events.push(event),
            Err(_) => {
                corrupt_lines = corrupt_lines.saturating_add(1);
            }
        }
    }

    SpoolReadOutcome {
        events,
        corrupt_lines,
        bytes_read: bytes.len(),
    }
}

fn trim_ascii_whitespace(mut slice: &[u8]) -> &[u8] {
    while let Some((first, rest)) = slice.split_first() {
        if first.is_ascii_whitespace() {
            slice = rest;
        } else {
            break;
        }
    }
    while let Some((last, rest)) = slice.split_last() {
        if last.is_ascii_whitespace() {
            slice = rest;
        } else {
            break;
        }
    }
    slice
}

fn serialize_events_jsonl(path: &Path, events: &[WorkEvent]) -> Result<Vec<u8>, StoreError> {
    let mut out = Vec::with_capacity(events.len().saturating_mul(72));
    for ev in events {
        serde_json::to_writer(&mut out, ev).map_err(|source| StoreError::Json {
            path: path.to_path_buf(),
            source,
        })?;
        out.push(b'\n');
    }
    Ok(out)
}
