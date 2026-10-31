//! Opt-in `activity` signal idle-time probes, deterministic [`FakeIdleSource`], output parsers,
//! and consent-gated 60-second tick sampler (PLAN §5, §7 & §10 T10).
//!
//! Architectural decision and verified platform documentation:
//! - `docs/decisions/003-macos-idle-time-source.md`
//! - `docs/platform/idle-time.md`

use crate::consent::{is_event_consented, is_signal_enabled, record_if_consented};
use std::collections::VecDeque;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tg_core::clock::{Timestamp, TICK_SECS};
use tg_store::config::{AppConfig, SignalKind};
use tg_store::error::StoreError;
use tg_store::spool::{EventSpool, WorkEvent};

/// Maximum elapsed user-input idle duration (`60 s`, one garden tick) for a completed tick
/// to earn the `sun` active bonus (`PLAN.md` §4.1 & §5).
pub const ACTIVITY_ACTIVE_WINDOW: Duration = Duration::from_secs(TICK_SECS as u64);

/// Returns `true` when `idle` is strictly less than [`ACTIVITY_ACTIVE_WINDOW`] (`60 s`).
#[must_use]
pub fn is_idle_duration_active(idle: Duration) -> bool {
    idle < ACTIVITY_ACTIVE_WINDOW
}

/// Constructs a privacy-safe [`WorkEvent::Activity`] at `ts` from a sampled `idle` duration.
#[must_use]
pub fn activity_event_from_idle(ts: Timestamp, idle: Duration) -> WorkEvent {
    WorkEvent::Activity {
        ts,
        active: is_idle_duration_active(idle),
    }
}

/// Read-only adapter querying elapsed local user-input idle duration (`PLAN.md` §5 & §10 T10).
pub trait IdleSource {
    /// Returns the duration since the user's most recent local keyboard/pointer input,
    /// or `None` when the host session cannot report idle duration.
    fn sample_idle_duration(&self) -> Option<Duration>;
}

/// Deterministic test double implementing [`IdleSource`] with an atomic poll counter.
#[derive(Debug)]
pub struct FakeIdleSource {
    state: Mutex<FakeIdleState>,
    calls: AtomicUsize,
}

#[derive(Debug)]
struct FakeIdleState {
    queue: VecDeque<Option<Duration>>,
    fallback: Option<Duration>,
}

impl FakeIdleSource {
    /// Creates an idle source that always returns `sample`.
    #[must_use]
    pub fn constant(sample: Option<Duration>) -> Self {
        Self {
            state: Mutex::new(FakeIdleState {
                queue: VecDeque::new(),
                fallback: sample,
            }),
            calls: AtomicUsize::new(0),
        }
    }

    /// Creates an idle source simulating a headless/unsupported session (`None`).
    #[must_use]
    pub fn unavailable() -> Self {
        Self::constant(None)
    }

    /// Creates an idle source popping scripted `samples` in order, then returning `fallback`.
    #[must_use]
    pub fn sequence<I>(samples: I, fallback: Option<Duration>) -> Self
    where
        I: IntoIterator<Item = Option<Duration>>,
    {
        Self {
            state: Mutex::new(FakeIdleState {
                queue: samples.into_iter().collect(),
                fallback,
            }),
            calls: AtomicUsize::new(0),
        }
    }

    /// Updates the fallback idle duration returned once any scripted queue is drained.
    pub fn set_fallback(&self, sample: Option<Duration>) {
        let mut guard = self.state.lock().expect("FakeIdleSource mutex poisoned");
        guard.fallback = sample;
    }

    /// Appends a scripted sample to the back of the queue.
    pub fn push_sample(&self, sample: Option<Duration>) {
        let mut guard = self.state.lock().expect("FakeIdleSource mutex poisoned");
        guard.queue.push_back(sample);
    }

    /// Returns how many times [`IdleSource::sample_idle_duration`] has been invoked.
    #[must_use]
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl IdleSource for FakeIdleSource {
    fn sample_idle_duration(&self) -> Option<Duration> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let mut guard = self.state.lock().expect("FakeIdleSource mutex poisoned");
        guard.queue.pop_front().unwrap_or(guard.fallback)
    }
}

/// Safe-Rust host OS idle-time sampler (`PLAN.md` §5, ADR 003, `docs/platform/idle-time.md`).
///
/// - **macOS:** reads `"HIDIdleTime"` (nanoseconds) from `/usr/sbin/ioreg -c IOHIDSystem -d 4 -r`
///   with zero `unsafe` blocks and zero TCC permission prompts (serving as the standalone CLI
///   fallback alongside `Niwa.app`'s in-process `CGEventSource` sampler in T14).
/// - **Linux:** best-effort probes `org.gnome.Mutter.IdleMonitor.GetIdletime`, `xprintidle`
///   (when `$DISPLAY` is set), and `org.freedesktop.ScreenSaver.GetSessionIdleTime`, returning
///   `None` on headless or unsupported sessions.
#[derive(Copy, Clone, Debug, Default)]
pub struct PlatformIdleSource;

impl IdleSource for PlatformIdleSource {
    fn sample_idle_duration(&self) -> Option<Duration> {
        #[cfg(target_os = "macos")]
        {
            let stdout =
                run_command_stdout("/usr/sbin/ioreg", &["-c", "IOHIDSystem", "-d", "4", "-r"])?;
            parse_ioreg_hid_idle_ns(&stdout)
        }

        #[cfg(not(target_os = "macos"))]
        {
            if let Some(stdout) = run_command_stdout(
                "gdbus",
                &[
                    "call",
                    "--session",
                    "--dest",
                    "org.gnome.Mutter.IdleMonitor",
                    "--object-path",
                    "/org/gnome/Mutter/IdleMonitor/Core",
                    "--method",
                    "org.gnome.Mutter.IdleMonitor.GetIdletime",
                ],
            ) {
                if let Some(ms) = parse_gdbus_uint_tuple(&stdout) {
                    return Some(Duration::from_millis(ms));
                }
            }

            if std::env::var_os("DISPLAY").is_some_and(|val| !val.is_empty()) {
                if let Some(stdout) = run_command_stdout("xprintidle", &[]) {
                    if let Some(dur) = parse_xprintidle_ms(&stdout) {
                        return Some(dur);
                    }
                }
            }

            if let Some(stdout) = run_command_stdout(
                "gdbus",
                &[
                    "call",
                    "--session",
                    "--dest",
                    "org.freedesktop.ScreenSaver",
                    "--object-path",
                    "/ScreenSaver",
                    "--method",
                    "org.freedesktop.ScreenSaver.GetSessionIdleTime",
                ],
            ) {
                if let Some(raw_val) = parse_gdbus_uint_tuple(&stdout) {
                    let desktop = std::env::var("XDG_CURRENT_DESKTOP").ok();
                    return Some(freedesktop_screensaver_idle_duration(
                        raw_val,
                        desktop.as_deref(),
                    ));
                }
            }

            None
        }
    }
}

/// Parses `"HIDIdleTime" = <nanoseconds>` from macOS `ioreg -c IOHIDSystem -d 4 -r` output.
#[must_use]
pub fn parse_ioreg_hid_idle_ns(stdout: &str) -> Option<Duration> {
    for line in stdout.lines() {
        let trimmed = line.trim();
        let Some(idx) = trimmed.find("\"HIDIdleTime\"") else {
            continue;
        };
        let after_key = &trimmed[idx + "\"HIDIdleTime\"".len()..];
        let Some((_, rhs)) = after_key.split_once('=') else {
            continue;
        };
        let digits: String = rhs
            .trim()
            .chars()
            .take_while(|ch| ch.is_ascii_digit())
            .collect();
        if let Ok(nanos) = digits.parse::<u64>() {
            return Some(Duration::from_nanos(nanos));
        }
    }
    None
}

/// Extracts the unsigned integer scalar from a single-element `gdbus call` tuple
/// such as `(uint64 48250,)` or `(uint32 19,)` or `(0,)`.
#[must_use]
pub fn parse_gdbus_uint_tuple(stdout: &str) -> Option<u64> {
    let trimmed = stdout.trim();
    let inner = trimmed.strip_prefix('(')?.strip_suffix(')')?;
    let first_elem = inner.split(',').next()?.trim();
    let numeric_token = first_elem.split_whitespace().last()?;
    numeric_token.parse::<u64>().ok()
}

/// Converts a raw `org.freedesktop.ScreenSaver.GetSessionIdleTime` integer into a [`Duration`].
///
/// Per `docs/platform/idle-time.md`, the formal freedesktop spec defines `GetSessionIdleTime`
/// in seconds, whereas KDE Plasma's `kscreenlocker` implementation returns milliseconds.
#[must_use]
pub fn freedesktop_screensaver_idle_duration(
    raw_value: u64,
    xdg_current_desktop: Option<&str>,
) -> Duration {
    let is_kde =
        xdg_current_desktop.is_some_and(|desktop| desktop.to_ascii_lowercase().contains("kde"));
    if is_kde {
        Duration::from_millis(raw_value)
    } else {
        Duration::from_secs(raw_value)
    }
}

/// Parses millisecond stdout emitted by X11 `xprintidle`.
#[must_use]
pub fn parse_xprintidle_ms(stdout: &str) -> Option<Duration> {
    let ms = stdout.trim().parse::<u64>().ok()?;
    Some(Duration::from_millis(ms))
}

/// Outcome returned by [`sample_and_record_activity`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ActivitySampleOutcome {
    /// `signals.activity` is disabled (or consented after the completed tick boundary);
    /// [`IdleSource`] was not polled.
    Disabled,
    /// No garden save exists yet or `< 60 s` (`TICK_SECS`) elapsed since `last_tick_at`;
    /// [`IdleSource`] was not polled.
    NoCompletedTick,
    /// A consented [`WorkEvent::Activity`] already exists in `events.jsonl` for the latest
    /// completed tick window (for example written by `Niwa.app`); [`IdleSource`] was not polled.
    AlreadyRecorded,
    /// [`IdleSource`] returned `None` (unsupported session); no event fabricated.
    SourceUnavailable,
    /// Appended `WorkEvent::Activity { ts, active }` to `events.jsonl`.
    Recorded { ts: Timestamp, active: bool },
}

/// Samples [`IdleSource`] once for the latest completed 60-second tick window and appends
/// [`WorkEvent::Activity`] through [`record_if_consented`].
pub fn sample_and_record_activity(
    config: &AppConfig,
    spool: &EventSpool,
    existing_events: &[WorkEvent],
    last_tick_at: Option<Timestamp>,
    now: Timestamp,
    idle_source: &dyn IdleSource,
) -> Result<ActivitySampleOutcome, StoreError> {
    if !is_signal_enabled(config, SignalKind::Activity) {
        return Ok(ActivitySampleOutcome::Disabled);
    }

    let Some(last_tick) = last_tick_at else {
        return Ok(ActivitySampleOutcome::NoCompletedTick);
    };
    if now < last_tick {
        return Ok(ActivitySampleOutcome::NoCompletedTick);
    }

    let elapsed_secs = now.as_unix_secs().saturating_sub(last_tick.as_unix_secs());
    let total_ticks = u64::try_from(elapsed_secs / TICK_SECS).unwrap_or(0);
    if total_ticks == 0 {
        return Ok(ActivitySampleOutcome::NoCompletedTick);
    }

    let tick_end = last_tick.add_ticks(total_ticks);
    let tick_window_start = last_tick.add_ticks(total_ticks - 1);

    let probe_event = WorkEvent::Activity {
        ts: tick_end,
        active: false,
    };
    if !is_event_consented(config, probe_event) {
        return Ok(ActivitySampleOutcome::Disabled);
    }

    let already_queued = existing_events.iter().copied().any(|ev| {
        matches!(ev, WorkEvent::Activity { ts, .. } if ts > tick_window_start && ts <= now)
            && is_event_consented(config, ev)
    });
    if already_queued {
        return Ok(ActivitySampleOutcome::AlreadyRecorded);
    }

    let Some(idle_duration) = idle_source.sample_idle_duration() else {
        return Ok(ActivitySampleOutcome::SourceUnavailable);
    };

    let event = activity_event_from_idle(tick_end, idle_duration);
    let active = matches!(event, WorkEvent::Activity { active, .. } if active);
    let recorded = record_if_consented(config, spool, event)?;
    if !recorded {
        return Ok(ActivitySampleOutcome::Disabled);
    }

    Ok(ActivitySampleOutcome::Recorded {
        ts: tick_end,
        active,
    })
}

fn run_command_stdout(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}
