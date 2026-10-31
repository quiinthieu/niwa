//! Ultra-low-latency `niwa _ingest` fast path for background shell hook events
//! (PLAN §3.4, §5, §7 & §10 T12).
//!
//! Appends a single privacy-safe `WorkEvent::Shell` JSONL line with `O_APPEND` and
//! exits immediately—without loading simulation catalogs, touching `garden.json`,
//! or waiting on `garden.lock`.

use clap::Args;
use std::process::ExitCode;
use tg_core::clock::Clock;
use tg_signals::consent::{is_signal_enabled, record_if_consented};
use tg_signals::{EventSpool, SignalKind, WorkEvent};
use tg_store::clock::SystemClock;
use tg_store::config::ConfigStore;
use tg_store::paths::NiwaPaths;
use tg_store::spool::{DurationBucket, ShellExitClass};

#[derive(Args, Clone, Debug, PartialEq, Eq)]
pub struct IngestArgs {
    /// Exit status code of the completed command (`0` -> `ok`, non-zero -> `err`).
    #[arg(long, allow_hyphen_values = true)]
    pub exit: i32,

    /// Elapsed command wall duration in seconds (mapped to `instant|fast|medium|long`).
    #[arg(long, allow_hyphen_values = true)]
    pub dur: String,
}

/// Appends one consented [`WorkEvent::Shell`] line to `events.jsonl` without acquiring `garden.lock`.
pub fn run_ingest(paths: &NiwaPaths, clock: &dyn Clock, args: &IngestArgs) -> anyhow::Result<bool> {
    let dur_secs = parse_duration_secs(&args.dur)?;
    let config = ConfigStore::new(paths.clone()).load()?;
    if !is_signal_enabled(&config, SignalKind::Shell) {
        return Ok(false);
    }

    let event = WorkEvent::Shell {
        ts: clock.now(),
        status: ShellExitClass::from_exit_code(args.exit),
        dur_bucket: DurationBucket::from_duration_secs(dur_secs),
    };
    let spool = EventSpool::new(paths.clone());
    Ok(record_if_consented(&config, &spool, event)?)
}

/// Direct `std::env::args_os()` fast path bypassing full `clap` command-tree construction
/// when invoked as `niwa _ingest --exit <code> --dur <secs>`.
///
/// Returns `None` for any non-`_ingest` command or when `--help` / unknown flags are passed
/// so `clap` handles standard help and usage diagnostics.
#[must_use]
pub fn try_run_fast_cli_from_env() -> Option<ExitCode> {
    let mut raw_args = std::env::args_os();
    let _bin = raw_args.next()?;
    let first = raw_args.next()?;

    // Support optional leading `--no-color` before `_ingest`, otherwise exit fast in ~100 ns.
    let rest_after_subcmd: Vec<String> = if first == "_ingest" {
        raw_args
            .map(|os| os.into_string().ok())
            .collect::<Option<Vec<_>>>()?
    } else if first == "--no-color" {
        let sub = raw_args.next()?;
        if sub != "_ingest" {
            return None;
        }
        raw_args
            .map(|os| os.into_string().ok())
            .collect::<Option<Vec<_>>>()?
    } else {
        return None;
    };

    let parsed = parse_fast_ingest_tokens(&rest_after_subcmd)?;
    let result = NiwaPaths::resolve()
        .map_err(anyhow::Error::from)
        .and_then(|paths| run_ingest(&paths, &SystemClock, &parsed));

    Some(match result {
        Ok(_) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    })
}

fn parse_fast_ingest_tokens(tokens: &[String]) -> Option<IngestArgs> {
    let mut exit_val: Option<i32> = None;
    let mut dur_val: Option<String> = None;
    let mut idx = 0_usize;

    while idx < tokens.len() {
        let tok = tokens[idx].as_str();
        match tok {
            "--no-color" => {
                idx += 1;
            }
            "--exit" => {
                let raw = tokens.get(idx + 1)?;
                exit_val = Some(raw.trim().parse::<i32>().ok()?);
                idx += 2;
            }
            "--dur" => {
                let raw = tokens.get(idx + 1)?;
                dur_val = Some(raw.clone());
                idx += 2;
            }
            _ if tok.starts_with("--exit=") => {
                let raw = tok.strip_prefix("--exit=")?;
                exit_val = Some(raw.trim().parse::<i32>().ok()?);
                idx += 1;
            }
            _ if tok.starts_with("--dur=") => {
                let raw = tok.strip_prefix("--dur=")?;
                dur_val = Some(raw.to_string());
                idx += 1;
            }
            _ => return None,
        }
    }

    Some(IngestArgs {
        exit: exit_val?,
        dur: dur_val?,
    })
}

pub fn parse_duration_secs(raw: &str) -> anyhow::Result<f64> {
    let trimmed = raw.trim();
    let secs = if trimmed.contains(',') {
        let normalized = trimmed.replace(',', ".");
        normalized.parse::<f64>()
    } else {
        trimmed.parse::<f64>()
    }
    .map_err(|_| anyhow::anyhow!("invalid --dur value '{raw}': expected duration in seconds"))?;

    if secs.is_nan() {
        anyhow::bail!("invalid --dur value '{raw}': duration cannot be NaN");
    }
    Ok(secs)
}
