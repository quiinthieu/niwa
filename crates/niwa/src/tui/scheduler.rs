//! Power-budget frame scheduler enforcing the `<= 4 fps` cap and `0 fps` idle/unfocused drop
//! (PLAN §0.7, §4.3, §7 & §10 T08).

use std::time::Duration;
use tg_core::clock::TICK_SECS;
use tg_store::config::{MAX_FPS, MIN_FPS};

/// Power budget (PLAN §4.3 & §7): after 30 s with no user input, the TUI drops to `0 fps`
/// so an unattended terminal performs zero frame redraws.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// Power budget (PLAN §2, §4.1 & §7): garden simulation ticks advance every 60 s; while idle
/// at `0 fps`, the event loop blocks until the next 60 s tick boundary instead of waking per frame.
pub const SIM_TICK_INTERVAL: Duration = Duration::from_secs(TICK_SECS as u64);

/// Pure monotonic frame scheduler governing TUI redraw cadence and `crossterm::event::poll` sleeps.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameScheduler {
    fps: u8,
    frame_interval: Duration,
    focused: bool,
    last_input_at: Duration,
    last_draw_at: Option<Duration>,
}

impl FrameScheduler {
    /// Constructs a scheduler clamped to Niwa's `1..=4` fps budget (PLAN §7).
    #[must_use]
    pub fn new(configured_fps: u8) -> Self {
        let fps = configured_fps.clamp(MIN_FPS, MAX_FPS);
        // Power budget (PLAN §4.3 & §7): cap active TUI redraws at `1..=4` fps (`250 ms..=1000 ms`
        // minimum interval between draws) while focused and active.
        let frame_interval = Duration::from_millis(1_000 / u64::from(fps));
        Self {
            fps,
            frame_interval,
            focused: true,
            last_input_at: Duration::ZERO,
            last_draw_at: None,
        }
    }

    #[must_use]
    pub const fn fps(&self) -> u8 {
        self.fps
    }

    /// Updates the configured active frame-rate cap (`1..=4` fps) in place when `fps` is changed
    /// in the TUI Settings screen (`PLAN.md` §6, §7 & §10 T15).
    pub fn set_fps(&mut self, configured_fps: u8) {
        let fps = configured_fps.clamp(MIN_FPS, MAX_FPS);
        self.fps = fps;
        self.frame_interval = Duration::from_millis(1_000 / u64::from(fps));
    }

    #[must_use]
    pub const fn frame_interval(&self) -> Duration {
        self.frame_interval
    }

    #[must_use]
    pub const fn is_focused(&self) -> bool {
        self.focused
    }

    /// Returns `true` when the terminal is unfocused or `>= 30 s` (`IDLE_TIMEOUT`) have elapsed
    /// since the last user input event (PLAN §4.3 & §7).
    #[must_use]
    pub fn is_idle(&self, mono_now: Duration) -> bool {
        !self.focused || mono_now.saturating_sub(self.last_input_at) >= IDLE_TIMEOUT
    }

    /// Effective frame-rate cap at `mono_now`: `0` when idle/unfocused, `1..=4` while active.
    #[must_use]
    pub fn effective_fps(&self, mono_now: Duration) -> u8 {
        if self.is_idle(mono_now) {
            0
        } else {
            self.fps
        }
    }

    /// Records a user input event (key/resize) at `mono_now`, waking from idle if focused.
    pub fn on_user_input(&mut self, mono_now: Duration) {
        self.last_input_at = mono_now;
    }

    /// Immediately drops to `0 fps` when the terminal loses window focus (PLAN §4.3).
    pub fn on_focus_lost(&mut self) {
        self.focused = false;
    }

    /// Restores active rendering when the terminal regains focus (PLAN §4.3).
    pub fn on_focus_gained(&mut self, mono_now: Duration) {
        self.focused = true;
        self.last_input_at = mono_now;
    }

    /// Returns `true` if a frame redraw is permitted at `mono_now` under the §7 power budget.
    #[must_use]
    pub fn should_draw(&self, mono_now: Duration) -> bool {
        if self.is_idle(mono_now) {
            return false;
        }
        match self.last_draw_at {
            None => true,
            Some(prev) => mono_now.saturating_sub(prev) >= self.frame_interval,
        }
    }

    /// Marks that a frame was drawn at `mono_now`.
    pub fn mark_drawn(&mut self, mono_now: Duration) {
        self.last_draw_at = Some(mono_now);
    }

    /// Computes the next `crossterm::event::poll` sleep duration.
    ///
    /// Power budget (PLAN §7):
    /// - When idle or unfocused (`0 fps`), sleeps for the entire `next_tick_in` (up to `60 s`)
    ///   with zero intermediate animation wakes.
    /// - When active (`1..=4 fps`), wakes at the earliest of the next frame slot
    ///   (`>= 250 ms` cadence), the `30 s` idle cutoff, or the next `60 s` simulation tick.
    #[must_use]
    pub fn next_poll_timeout(&self, mono_now: Duration, next_tick_in: Duration) -> Duration {
        if self.is_idle(mono_now) {
            return next_tick_in;
        }
        let until_next_frame = match self.last_draw_at {
            None => Duration::ZERO,
            Some(prev) => self
                .frame_interval
                .saturating_sub(mono_now.saturating_sub(prev)),
        };
        let until_idle = IDLE_TIMEOUT.saturating_sub(mono_now.saturating_sub(self.last_input_at));
        until_next_frame.min(until_idle).min(next_tick_in)
    }
}
