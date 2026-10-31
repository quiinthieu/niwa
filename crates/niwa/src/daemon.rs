//! `niwa daemon` 60-second background tick loop and per-user `launchd`/`systemd` installer
//! handlers (PLAN §0.7, §3.4, §5, §7 & §10 T10).

use crate::runtime::GardenSession;
use std::io::Write;
use std::path::Path;
use std::time::Duration;
use tg_core::clock::{Clock, TICK_SECS};
use tg_signals::activity::{sample_and_record_activity, ActivitySampleOutcome, IdleSource};
use tg_signals::consent::{any_signal_enabled, is_signal_enabled};
use tg_signals::git::{GitLogRunner, GitPollMode, ProcessGitLogRunner};
use tg_signals::{EventSpool, SignalKind};
use tg_store::config::ConfigStore;
use tg_store::daemon_units::{
    install_daemon_unit, uninstall_daemon_unit, DaemonUnitPaths, InstallDaemonUnitOutcome,
    UninstallDaemonUnitOutcome,
};
use tg_store::error::StoreError;
use tg_store::paths::{NiwaPaths, PlatformKind};
use tg_store::save::GardenStore;

/// Coalesced background wake cadence (`60 s`, matching [`TICK_SECS`]).
///
/// Power-budget justification (`PLAN.md` §0.7 & §7):
/// - The daemon wakes at most once every 60 seconds (one garden tick), performs zero filesystem
///   polling, and skips waking/sleeping entirely (exiting `0`) whenever no opt-in work signal is
///   enabled in `config.toml`.
pub const DAEMON_TICK_INTERVAL: Duration = Duration::from_secs(TICK_SECS as u64);

/// Injectable sleep strategy separating the 60-second timer from deterministic unit/integration tests.
pub trait DaemonSleeper {
    fn sleep(&self, duration: Duration);
}

/// Production [`DaemonSleeper`] blocking the background daemon thread for [`DAEMON_TICK_INTERVAL`].
#[derive(Copy, Clone, Debug, Default)]
pub struct ThreadSleeper;

impl DaemonSleeper for ThreadSleeper {
    fn sleep(&self, duration: Duration) {
        // Power budget (PLAN §0.7 & §7): single 60 s sleep per garden tick (`DAEMON_TICK_INTERVAL`),
        // allowing the OS kernel (`launchd` `ProcessType=Background` / Linux timer slack) to
        // coalesce wakes with zero busy-waiting and zero filesystem polling.
        std::thread::sleep(duration);
    }
}

impl<F> DaemonSleeper for F
where
    F: Fn(Duration),
{
    fn sleep(&self, duration: Duration) {
        (self)(duration);
    }
}

/// Reason [`run_daemon_loop`] exited cleanly.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DaemonStopReason {
    /// No opt-in work signal was enabled in `config.toml`; exited immediately with zero wakes
    /// (`PLAN.md` §7 & §10 T10 AC 3).
    SkippedNoSignals,
    /// Completed the caller-requested `max_wakes` bound (`--once` or `--max-ticks N`).
    ReachedMaxWakes,
}

/// Execution telemetry returned by [`run_daemon_loop`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct DaemonRunSummary {
    pub stop_reason: DaemonStopReason,
    pub wakes_executed: u64,
    pub sleeps_performed: u64,
    pub ticks_simulated: u64,
    pub active_samples_recorded: u64,
    pub idle_samples_recorded: u64,
}

/// Runs the 60-second `niwa daemon` loop (`PLAN.md` §3.4, §7 & §10 T10–T11).
///
/// Before acquiring `garden.lock`, polling [`IdleSource`], touching `garden.json`, or sleeping,
/// loads `config.toml` and verifies [`any_signal_enabled`]. When no signal is enabled, returns
/// [`DaemonStopReason::SkippedNoSignals`] immediately so `launchd` (`SuccessfulExit = false`)
/// and `systemd` (`Restart=on-failure`) leave the daemon stopped with zero background wakes.
pub fn run_daemon_loop(
    paths: NiwaPaths,
    clock: &dyn Clock,
    idle_source: &dyn IdleSource,
    sleeper: &dyn DaemonSleeper,
    max_wakes: Option<u64>,
) -> Result<DaemonRunSummary, StoreError> {
    run_daemon_loop_with_git(
        paths,
        clock,
        idle_source,
        &ProcessGitLogRunner::default(),
        sleeper,
        max_wakes,
    )
}

/// Variant of [`run_daemon_loop`] accepting an injected [`GitLogRunner`] so integration tests can
/// assert the 5-minute periodic `git log` throttle across 60-second daemon wakes.
pub fn run_daemon_loop_with_git(
    paths: NiwaPaths,
    clock: &dyn Clock,
    idle_source: &dyn IdleSource,
    git_runner: &dyn GitLogRunner,
    sleeper: &dyn DaemonSleeper,
    max_wakes: Option<u64>,
) -> Result<DaemonRunSummary, StoreError> {
    let config_store = ConfigStore::new(paths.clone());
    let garden_store = GardenStore::new(paths.clone());
    let spool = EventSpool::new(paths.clone());

    let mut wakes_executed = 0_u64;
    let mut sleeps_performed = 0_u64;
    let mut ticks_simulated = 0_u64;
    let mut active_samples_recorded = 0_u64;
    let mut idle_samples_recorded = 0_u64;

    if max_wakes == Some(0) {
        return Ok(DaemonRunSummary {
            stop_reason: DaemonStopReason::ReachedMaxWakes,
            wakes_executed,
            sleeps_performed,
            ticks_simulated,
            active_samples_recorded,
            idle_samples_recorded,
        });
    }

    loop {
        let config = config_store.load()?;
        if !any_signal_enabled(&config) {
            return Ok(DaemonRunSummary {
                stop_reason: DaemonStopReason::SkippedNoSignals,
                wakes_executed,
                sleeps_performed,
                ticks_simulated,
                active_samples_recorded,
                idle_samples_recorded,
            });
        }

        let now = clock.now();
        let prev_last_tick_at = garden_store.load()?.map(|loaded| loaded.save.last_tick_at);

        if is_signal_enabled(&config, SignalKind::Activity) {
            let spool_read = spool.read()?;
            if let ActivitySampleOutcome::Recorded { active, .. } = sample_and_record_activity(
                &config,
                &spool,
                &spool_read.events,
                prev_last_tick_at,
                now,
                idle_source,
            )? {
                if active {
                    active_samples_recorded = active_samples_recorded.saturating_add(1);
                } else {
                    idle_samples_recorded = idle_samples_recorded.saturating_add(1);
                }
            }
        }

        let session = GardenSession::open_with_git(
            paths.clone(),
            clock,
            None,
            GitPollMode::PeriodicInterval,
            git_runner,
        )?;
        if let Some(notice) = session.recovery_notice() {
            eprintln!("warning: {notice}");
        }
        if let Some(away) = session.last_away() {
            ticks_simulated = ticks_simulated.saturating_add(away.ticks_simulated);
        }

        wakes_executed = wakes_executed.saturating_add(1);
        if max_wakes.is_some_and(|limit| wakes_executed >= limit) {
            return Ok(DaemonRunSummary {
                stop_reason: DaemonStopReason::ReachedMaxWakes,
                wakes_executed,
                sleeps_performed,
                ticks_simulated,
                active_samples_recorded,
                idle_samples_recorded,
            });
        }

        // Power budget (PLAN §0.7 & §7): one coalesced wake per 60 s (`DAEMON_TICK_INTERVAL`).
        sleeper.sleep(DAEMON_TICK_INTERVAL);
        sleeps_performed = sleeps_performed.saturating_add(1);
    }
}

/// Installs the per-user `launchd` plist or `systemd` user service via `tg-store` and prints
/// activation instructions.
pub fn run_daemon_install(
    unit_paths: &DaemonUnitPaths,
    binary_path: &Path,
    writer: &mut dyn Write,
) -> anyhow::Result<InstallDaemonUnitOutcome> {
    let outcome = install_daemon_unit(unit_paths, binary_path)?;
    writeln!(
        writer,
        "Installed user daemon unit '{}' at {}",
        outcome.unit_identifier,
        outcome.unit_path.display()
    )?;
    match outcome.platform {
        PlatformKind::MacOs => {
            writeln!(
                writer,
                "To load now: launchctl bootstrap \"gui/$(id -u)\" \"{}\"",
                outcome.unit_path.display()
            )?;
        }
        PlatformKind::Linux => {
            writeln!(
                writer,
                "To enable now: systemctl --user daemon-reload && systemctl --user enable --now {}",
                outcome.unit_identifier
            )?;
        }
    }
    Ok(outcome)
}

/// Removes the per-user `launchd` plist or `systemd` user service via `tg-store` (idempotent).
pub fn run_daemon_uninstall(
    unit_paths: &DaemonUnitPaths,
    writer: &mut dyn Write,
) -> anyhow::Result<UninstallDaemonUnitOutcome> {
    let outcome = uninstall_daemon_unit(unit_paths)?;
    if outcome.removed {
        writeln!(
            writer,
            "Removed user daemon unit '{}' from {}",
            outcome.unit_identifier,
            outcome.unit_path.display()
        )?;
    } else {
        writeln!(
            writer,
            "User daemon unit '{}' is not installed ({})",
            outcome.unit_identifier,
            outcome.unit_path.display()
        )?;
    }
    Ok(outcome)
}

/// Prints the per-user daemon unit installation state and enabled signal wake status.
pub fn run_daemon_status(
    paths: &NiwaPaths,
    unit_paths: &DaemonUnitPaths,
    writer: &mut dyn Write,
) -> anyhow::Result<()> {
    let config = ConfigStore::new(paths.clone()).load()?;
    let install_state = if unit_paths.is_installed() {
        "installed"
    } else {
        "not installed"
    };
    let enabled: Vec<&'static str> = SignalKind::ALL
        .into_iter()
        .filter(|&kind| is_signal_enabled(&config, kind))
        .map(SignalKind::as_str)
        .collect();
    let wake_policy = if enabled.is_empty() {
        "none (daemon skips waking — 0 background wakes)".to_string()
    } else {
        format!("{} (60 s coalesced loop)", enabled.join(", "))
    };

    writeln!(writer, "Unit:    {}", unit_paths.unit_identifier())?;
    writeln!(writer, "Path:    {}", unit_paths.unit_path().display())?;
    writeln!(writer, "State:   {install_state}")?;
    writeln!(writer, "Signals: {wake_policy}")?;
    Ok(())
}
