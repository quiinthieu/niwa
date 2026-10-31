//! Pure event-driven TUI state machine (`TuiApp`) testable on `ratatui::backend::TestBackend`
//! with an injected [`tg_core::clock::Clock`] (PLAN §4.2, §4.3, §7 & §10 T08).

use crate::runtime::{resolve_color_tier, ActionError, GardenSession};
use crate::tui::scheduler::FrameScheduler;
use crate::tui::settings::{SettingsField, SettingsState};
use crate::tui::ui::draw_ui;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::backend::Backend;
use ratatui::Terminal;
use std::time::Duration;
use tg_core::catch_up::AwaySummary;
use tg_core::clock::Clock;
use tg_core::species::Rarity;
use tg_render::ColorTier;
use tg_signals::SignalKind;
use tg_store::error::StoreError;

/// Discriminant of the currently open modal overlay (if any).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum OverlayKind {
    AwaySummary,
    SeedPicker,
    ConfirmCompost,
    Almanac,
    Settings,
    SignalConsent,
    Help,
}

#[derive(Clone, Debug)]
pub(crate) enum Overlay {
    AwaySummary(AwaySummary),
    SeedPicker {
        cursor: usize,
    },
    ConfirmCompost {
        plot_index: usize,
    },
    Almanac {
        cursor: usize,
    },
    Settings(SettingsState),
    SignalConsent {
        signal: SignalKind,
        return_cursor: usize,
    },
    Help,
}

impl Overlay {
    const fn kind(&self) -> OverlayKind {
        match self {
            Self::AwaySummary(_) => OverlayKind::AwaySummary,
            Self::SeedPicker { .. } => OverlayKind::SeedPicker,
            Self::ConfirmCompost { .. } => OverlayKind::ConfirmCompost,
            Self::Almanac { .. } => OverlayKind::Almanac,
            Self::Settings(_) => OverlayKind::Settings,
            Self::SignalConsent { .. } => OverlayKind::SignalConsent,
            Self::Help => OverlayKind::Help,
        }
    }
}

/// Entry displayed inside the Seed Pouch picker modal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeedChoice {
    pub species_id: String,
    pub display_name: String,
    pub rarity: Rarity,
    pub count: u32,
    pub can_sprout_now: bool,
}

/// Interactive TUI controller owning the live [`GardenSession`] and [`FrameScheduler`].
#[derive(Debug)]
pub struct TuiApp {
    session: GardenSession,
    scheduler: FrameScheduler,
    color_tier: ColorTier,
    ambient_phase: u8,
    selected_plot: usize,
    move_source: Option<usize>,
    overlay: Option<Overlay>,
    status_message: String,
    should_quit: bool,
}

impl TuiApp {
    /// Initializes a `TuiApp`, waking any dormant garden on open and queuing the
    /// `"While you were away"` modal when offline progress occurred.
    pub fn new(
        mut session: GardenSession,
        color_tier: ColorTier,
        clock: &dyn Clock,
    ) -> Result<Self, StoreError> {
        let away = session.take_last_away();
        session.wake_on_tui_open(clock.now())?;
        let scheduler = FrameScheduler::new(session.config().fps);
        let status_message = if let Some(notice) = session.recovery_notice() {
            notice.to_string()
        } else {
            "Welcome to Niwa — [p]lant [h]arvest [m]ove [c]ompost [a]lmanac [s]ettings [?]help [q]uit"
                .to_string()
        };

        Ok(Self {
            session,
            scheduler,
            color_tier,
            ambient_phase: 0,
            selected_plot: 0,
            move_source: None,
            overlay: away.map(Overlay::AwaySummary),
            status_message,
            should_quit: false,
        })
    }

    #[must_use]
    pub const fn session(&self) -> &GardenSession {
        &self.session
    }

    #[must_use]
    pub const fn scheduler(&self) -> &FrameScheduler {
        &self.scheduler
    }

    #[must_use]
    pub const fn color_tier(&self) -> ColorTier {
        self.color_tier
    }

    #[must_use]
    pub const fn ambient_phase(&self) -> u8 {
        self.ambient_phase
    }

    #[must_use]
    pub const fn selected_plot(&self) -> usize {
        self.selected_plot
    }

    #[must_use]
    pub const fn move_source(&self) -> Option<usize> {
        self.move_source
    }

    #[must_use]
    pub const fn is_moving(&self) -> bool {
        self.move_source.is_some()
    }

    #[must_use]
    pub fn overlay_kind(&self) -> Option<OverlayKind> {
        self.overlay.as_ref().map(Overlay::kind)
    }

    /// Returns the currently highlighted [`SettingsField`] when the Settings modal is open.
    #[must_use]
    pub fn selected_settings_field(&self) -> Option<SettingsField> {
        match &self.overlay {
            Some(Overlay::Settings(state)) => Some(state.selected_field()),
            _ => None,
        }
    }

    #[must_use]
    pub(crate) const fn overlay(&self) -> Option<&Overlay> {
        self.overlay.as_ref()
    }

    #[must_use]
    pub fn status_message(&self) -> &str {
        &self.status_message
    }

    #[must_use]
    pub const fn should_quit(&self) -> bool {
        self.should_quit
    }

    /// Sorted list of seeds currently available in the player's seed pouch.
    #[must_use]
    pub fn available_seeds(&self) -> Vec<SeedChoice> {
        let season = self.session.current_season();
        self.session
            .save()
            .inventory
            .iter()
            .filter(|(_, &count)| count > 0)
            .map(|(id, &count)| {
                let (display_name, rarity, can_sprout_now) =
                    self.session.catalog().get(id).map_or_else(
                        || (id.clone(), Rarity::Common, true),
                        |sp| (sp.display_name.clone(), sp.rarity, sp.can_sprout_in(season)),
                    );
                SeedChoice {
                    species_id: id.clone(),
                    display_name,
                    rarity,
                    count,
                    can_sprout_now,
                }
            })
            .collect()
    }

    /// Applies any 60-second active ticks elapsed on `clock` while the TUI is open
    /// (PLAN §4.1 & §10 T08: "The TUI counts as an active signal while open").
    pub fn sync_ticks(&mut self, clock: &dyn Clock) -> Result<usize, StoreError> {
        self.session.advance_open_tui_ticks(clock.now())
    }

    /// Processes a terminal [`Event`] (keys, focus changes, resize) and updates [`FrameScheduler`].
    pub fn handle_event(
        &mut self,
        event: &Event,
        mono_now: Duration,
        clock: &dyn Clock,
    ) -> Result<(), StoreError> {
        match event {
            Event::Key(key) => self.handle_key(*key, mono_now, clock)?,
            Event::FocusLost => {
                self.scheduler.on_focus_lost();
            }
            Event::FocusGained => {
                self.scheduler.on_focus_gained(mono_now);
            }
            Event::Resize(_, _) | Event::Mouse(_) | Event::Paste(_) => {
                self.scheduler.on_user_input(mono_now);
            }
        }
        Ok(())
    }

    /// Processes a single keyboard event at `mono_now`.
    pub fn handle_key(
        &mut self,
        key: KeyEvent,
        mono_now: Duration,
        clock: &dyn Clock,
    ) -> Result<(), StoreError> {
        if key.kind == KeyEventKind::Release {
            return Ok(());
        }
        self.scheduler.on_user_input(mono_now);

        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('c' | 'C'))
        {
            self.should_quit = true;
            return Ok(());
        }

        if let Some(overlay) = self.overlay.clone() {
            return self.handle_overlay_key(overlay, key, clock);
        }

        if let Some(source_plot) = self.move_source {
            return self.handle_move_mode_key(source_plot, key, clock);
        }

        self.handle_grid_key(key, clock)
    }

    /// Executes one scheduled loop step: handles `event`, syncs elapsed 60 s open-TUI ticks,
    /// and draws a frame ONLY if permitted by [`FrameScheduler::should_draw`].
    pub fn step<B: Backend>(
        &mut self,
        event: Option<Event>,
        mono_now: Duration,
        clock: &dyn Clock,
        terminal: &mut Terminal<B>,
    ) -> anyhow::Result<bool>
    where
        B::Error: Send + Sync + 'static,
    {
        if let Some(ev) = event {
            self.handle_event(&ev, mono_now, clock)?;
        }
        self.sync_ticks(clock)?;

        if self.scheduler.should_draw(mono_now) {
            self.render(terminal)?;
            self.scheduler.mark_drawn(mono_now);
            self.ambient_phase = (self.ambient_phase + 1) % 4;
            return Ok(true);
        }
        Ok(false)
    }

    /// Renders the current `TuiApp` view directly onto `terminal`.
    pub fn render<B: Backend>(&self, terminal: &mut Terminal<B>) -> anyhow::Result<()>
    where
        B::Error: Send + Sync + 'static,
    {
        terminal.draw(|frame| draw_ui(frame, self))?;
        Ok(())
    }

    fn handle_overlay_key(
        &mut self,
        overlay: Overlay,
        key: KeyEvent,
        clock: &dyn Clock,
    ) -> Result<(), StoreError> {
        match overlay {
            Overlay::AwaySummary(_) => {
                if matches!(
                    key.code,
                    KeyCode::Enter | KeyCode::Esc | KeyCode::Char(' ' | 'q')
                ) {
                    self.overlay = None;
                }
            }
            Overlay::SeedPicker { mut cursor } => {
                let choices = self.available_seeds();
                if choices.is_empty() {
                    self.overlay = None;
                    return Ok(());
                }
                if cursor >= choices.len() {
                    cursor = choices.len() - 1;
                }
                match key.code {
                    KeyCode::Up | KeyCode::Char('k') => {
                        cursor = cursor.saturating_sub(1);
                        self.overlay = Some(Overlay::SeedPicker { cursor });
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if cursor + 1 < choices.len() {
                            cursor += 1;
                        }
                        self.overlay = Some(Overlay::SeedPicker { cursor });
                    }
                    KeyCode::Enter => {
                        let chosen = &choices[cursor];
                        match self.session.plant(
                            self.selected_plot,
                            &chosen.species_id,
                            clock.now(),
                        ) {
                            Ok(outcome) => {
                                self.status_message = format!(
                                    "Planted {} in Plot {}.",
                                    outcome.display_name,
                                    outcome.plot_index + 1
                                );
                            }
                            Err(ActionError::Store(err)) => return Err(err),
                            Err(err) => {
                                self.status_message = format!("Could not plant: {err}");
                            }
                        }
                        self.overlay = None;
                    }
                    KeyCode::Esc | KeyCode::Char('q') => {
                        self.overlay = None;
                    }
                    _ => {}
                }
            }
            Overlay::ConfirmCompost { plot_index } => match key.code {
                KeyCode::Char('y' | 'Y') | KeyCode::Enter => {
                    match self.session.compost(plot_index, clock.now()) {
                        Ok(outcome) => {
                            self.status_message = format!(
                                "Composted {} -> received 1 {} seed: {}.",
                                outcome.composted_display_name,
                                outcome.rarity.label(),
                                outcome.returned_display_name
                            );
                        }
                        Err(ActionError::Store(err)) => return Err(err),
                        Err(err) => {
                            self.status_message = format!("Could not compost: {err}");
                        }
                    }
                    self.overlay = None;
                }
                KeyCode::Char('n' | 'N') | KeyCode::Esc => {
                    self.overlay = None;
                    self.status_message = "Compost cancelled.".to_string();
                }
                _ => {}
            },
            Overlay::Almanac { mut cursor } => {
                let total = self.session.catalog().len();
                match key.code {
                    KeyCode::Up | KeyCode::Char('k') => {
                        cursor = cursor.saturating_sub(1);
                        self.overlay = Some(Overlay::Almanac { cursor });
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if cursor + 1 < total {
                            cursor += 1;
                        }
                        self.overlay = Some(Overlay::Almanac { cursor });
                    }
                    KeyCode::PageUp => {
                        cursor = cursor.saturating_sub(5);
                        self.overlay = Some(Overlay::Almanac { cursor });
                    }
                    KeyCode::PageDown => {
                        cursor = (cursor + 5).min(total.saturating_sub(1));
                        self.overlay = Some(Overlay::Almanac { cursor });
                    }
                    KeyCode::Esc | KeyCode::Char('a' | 'q') => {
                        self.overlay = None;
                    }
                    _ => {}
                }
            }
            Overlay::Settings(state) => {
                self.handle_settings_key(state, key, clock)?;
            }
            Overlay::SignalConsent {
                signal,
                return_cursor,
            } => {
                self.handle_signal_consent_key(signal, return_cursor, key, clock)?;
            }
            Overlay::Help => {
                if matches!(
                    key.code,
                    KeyCode::Esc | KeyCode::Enter | KeyCode::Char('?' | 'q' | ' ')
                ) {
                    self.overlay = None;
                }
            }
        }
        Ok(())
    }

    fn handle_settings_key(
        &mut self,
        mut state: SettingsState,
        key: KeyEvent,
        clock: &dyn Clock,
    ) -> Result<(), StoreError> {
        let total = SettingsField::ALL.len();
        state.cursor = state.cursor.min(total - 1);
        let field = state.selected_field();

        if let Some(mut buffer) = state.edit_buffer.take() {
            match key.code {
                KeyCode::Esc => {
                    state.error_message = None;
                    self.overlay = Some(Overlay::Settings(state));
                }
                KeyCode::Backspace => {
                    buffer.pop();
                    state.edit_buffer = Some(buffer);
                    self.overlay = Some(Overlay::Settings(state));
                }
                KeyCode::Char(ch) if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    if matches!(ch, 'u' | 'U') {
                        buffer.clear();
                    }
                    state.edit_buffer = Some(buffer);
                    self.overlay = Some(Overlay::Settings(state));
                }
                KeyCode::Char(ch) => {
                    buffer.push(ch);
                    state.edit_buffer = Some(buffer);
                    self.overlay = Some(Overlay::Settings(state));
                }
                KeyCode::Enter => {
                    let trimmed = buffer.trim();
                    let value_to_write = if trimmed.is_empty() && field.supports_clear_to_none() {
                        "none"
                    } else {
                        trimmed
                    };
                    match self.apply_settings_write(field.key(), value_to_write, clock)? {
                        Ok(_) => {
                            state.error_message = None;
                        }
                        Err(validation_msg) => {
                            state.error_message = Some(validation_msg);
                            state.edit_buffer = Some(buffer);
                        }
                    }
                    self.overlay = Some(Overlay::Settings(state));
                }
                _ => {
                    state.edit_buffer = Some(buffer);
                    self.overlay = Some(Overlay::Settings(state));
                }
            }
            return Ok(());
        }

        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                state.cursor = state.cursor.saturating_sub(1);
                state.error_message = None;
                self.overlay = Some(Overlay::Settings(state));
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if state.cursor + 1 < total {
                    state.cursor += 1;
                }
                state.error_message = None;
                self.overlay = Some(Overlay::Settings(state));
            }
            KeyCode::PageUp => {
                state.cursor = state.cursor.saturating_sub(5);
                state.error_message = None;
                self.overlay = Some(Overlay::Settings(state));
            }
            KeyCode::PageDown => {
                state.cursor = (state.cursor + 5).min(total - 1);
                state.error_message = None;
                self.overlay = Some(Overlay::Settings(state));
            }
            KeyCode::Home => {
                state.cursor = 0;
                state.error_message = None;
                self.overlay = Some(Overlay::Settings(state));
            }
            KeyCode::End => {
                state.cursor = total - 1;
                state.error_message = None;
                self.overlay = Some(Overlay::Settings(state));
            }
            KeyCode::Left => {
                if let Some(signal) = field.signal_consent_kind() {
                    if self.session.config().signals.consent(signal).enabled {
                        let _ = self.apply_settings_write(field.key(), "false", clock)?;
                    }
                } else if let Some(next_val) = field.step_directional(self.session.config(), false)
                {
                    let _ = self.apply_settings_write(field.key(), &next_val, clock)?;
                }
                state.error_message = None;
                self.overlay = Some(Overlay::Settings(state));
            }
            KeyCode::Right => {
                if let Some(signal) = field.signal_consent_kind() {
                    if !self.session.config().signals.consent(signal).enabled {
                        self.overlay = Some(Overlay::SignalConsent {
                            signal,
                            return_cursor: state.cursor,
                        });
                        return Ok(());
                    }
                } else if let Some(next_val) = field.step_directional(self.session.config(), true) {
                    let _ = self.apply_settings_write(field.key(), &next_val, clock)?;
                }
                state.error_message = None;
                self.overlay = Some(Overlay::Settings(state));
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                if let Some(signal) = field.signal_consent_kind() {
                    if self.session.config().signals.consent(signal).enabled {
                        let _ = self.apply_settings_write(field.key(), "false", clock)?;
                        state.error_message = None;
                        self.overlay = Some(Overlay::Settings(state));
                    } else {
                        self.overlay = Some(Overlay::SignalConsent {
                            signal,
                            return_cursor: state.cursor,
                        });
                    }
                    return Ok(());
                }

                if key.code == KeyCode::Enter && field.supports_inline_edit() {
                    state.edit_buffer = Some(field.initial_edit_buffer(self.session.config()));
                    state.error_message = None;
                    self.overlay = Some(Overlay::Settings(state));
                    return Ok(());
                }

                if let Some(next_val) = field.cycle_on_activate(self.session.config()) {
                    let _ = self.apply_settings_write(field.key(), &next_val, clock)?;
                }
                state.error_message = None;
                self.overlay = Some(Overlay::Settings(state));
            }
            KeyCode::Delete | KeyCode::Backspace | KeyCode::Char('x')
                if field.supports_clear_to_none() =>
            {
                let _ = self.apply_settings_write(field.key(), "none", clock)?;
                state.error_message = None;
                self.overlay = Some(Overlay::Settings(state));
            }
            KeyCode::Esc | KeyCode::Char('s' | 'q') => {
                self.overlay = None;
            }
            _ => {
                self.overlay = Some(Overlay::Settings(state));
            }
        }
        Ok(())
    }

    fn handle_signal_consent_key(
        &mut self,
        signal: SignalKind,
        return_cursor: usize,
        key: KeyEvent,
        clock: &dyn Clock,
    ) -> Result<(), StoreError> {
        match key.code {
            KeyCode::Char('y' | 'Y') | KeyCode::Enter => {
                let key_name = format!("signals.{}.enabled", signal.as_str());
                let _ = self.apply_settings_write(&key_name, "true", clock)?;
                let ts = self
                    .session
                    .config()
                    .signals
                    .consent(signal)
                    .consented_at
                    .map_or(0, |t| t.as_unix_secs());
                self.status_message = format!(
                    "Enabled signal '{}' (consented_at = {ts}).",
                    signal.as_str()
                );
                self.overlay = Some(Overlay::Settings(SettingsState {
                    cursor: return_cursor,
                    edit_buffer: None,
                    error_message: None,
                }));
            }
            KeyCode::Char('n' | 'N') | KeyCode::Esc => {
                self.status_message = format!(
                    "Signal '{}' remains disabled (consent not given).",
                    signal.as_str()
                );
                self.overlay = Some(Overlay::Settings(SettingsState {
                    cursor: return_cursor,
                    edit_buffer: None,
                    error_message: None,
                }));
            }
            _ => {}
        }
        Ok(())
    }

    fn apply_settings_write(
        &mut self,
        key: &str,
        value: &str,
        clock: &dyn Clock,
    ) -> Result<Result<String, String>, StoreError> {
        match self.session.set_config_key(key, value, clock) {
            Ok(normalized) => {
                if key == "fps" {
                    self.scheduler.set_fps(self.session.config().fps);
                } else if key == "color" {
                    self.color_tier = resolve_color_tier(self.session.config(), false);
                }
                let shown = if normalized.is_empty() {
                    "none".to_string()
                } else {
                    normalized.clone()
                };
                self.status_message = format!("Updated {key} = {shown}");
                Ok(Ok(normalized))
            }
            Err(StoreError::InvalidConfigValue {
                key: err_key,
                reason,
                ..
            }) => {
                let msg = format!("Invalid {err_key}: {reason}");
                self.status_message.clone_from(&msg);
                Ok(Err(msg))
            }
            Err(err) => Err(err),
        }
    }

    fn handle_move_mode_key(
        &mut self,
        source_plot: usize,
        key: KeyEvent,
        clock: &dyn Clock,
    ) -> Result<(), StoreError> {
        if self.navigate_plot(key.code) {
            return Ok(());
        }
        match key.code {
            KeyCode::Esc => {
                self.move_source = None;
                self.status_message = "Move cancelled.".to_string();
            }
            KeyCode::Enter | KeyCode::Char('m') => {
                let dest = self.selected_plot;
                if dest == source_plot {
                    self.move_source = None;
                    self.status_message = format!("Kept plant in Plot {}.", source_plot + 1);
                    return Ok(());
                }
                match self.session.move_plant(source_plot, dest, clock.now()) {
                    Ok(()) => {
                        self.move_source = None;
                        self.status_message = format!(
                            "Moved plant from Plot {} to Plot {}.",
                            source_plot + 1,
                            dest + 1
                        );
                    }
                    Err(ActionError::Store(err)) => return Err(err),
                    Err(err) => {
                        self.status_message = format!("Cannot move to Plot {}: {err}", dest + 1);
                    }
                }
            }
            KeyCode::Char('q') => {
                self.should_quit = true;
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_grid_key(&mut self, key: KeyEvent, clock: &dyn Clock) -> Result<(), StoreError> {
        if self.navigate_plot(key.code) {
            return Ok(());
        }

        match key.code {
            KeyCode::Char('p') => self.open_seed_picker(),
            KeyCode::Char('h') => self.try_harvest_selected(clock)?,
            KeyCode::Enter => {
                let is_empty = self
                    .session
                    .garden()
                    .plots()
                    .get(self.selected_plot)
                    .is_some_and(tg_core::model::Plot::is_empty);
                if is_empty {
                    self.open_seed_picker();
                } else {
                    self.try_harvest_selected(clock)?;
                }
            }
            KeyCode::Char('c') => {
                let occupied = self
                    .session
                    .garden()
                    .plots()
                    .get(self.selected_plot)
                    .is_some_and(|p| !p.is_empty());
                if occupied {
                    self.overlay = Some(Overlay::ConfirmCompost {
                        plot_index: self.selected_plot,
                    });
                } else {
                    self.status_message = format!(
                        "Plot {} is empty — nothing to compost.",
                        self.selected_plot + 1
                    );
                }
            }
            KeyCode::Char('m') => {
                let occupied = self
                    .session
                    .garden()
                    .plots()
                    .get(self.selected_plot)
                    .is_some_and(|p| !p.is_empty());
                if occupied {
                    self.move_source = Some(self.selected_plot);
                    self.status_message = format!(
                        "Moving Plot {}: select an empty plot and press Enter (Esc to cancel).",
                        self.selected_plot + 1
                    );
                } else {
                    self.status_message = format!(
                        "Plot {} is empty — select a planted plot to move.",
                        self.selected_plot + 1
                    );
                }
            }
            KeyCode::Char('a') => {
                self.overlay = Some(Overlay::Almanac { cursor: 0 });
            }
            KeyCode::Char('s') => {
                self.overlay = Some(Overlay::Settings(SettingsState::default()));
            }
            KeyCode::Char('?') => {
                self.overlay = Some(Overlay::Help);
            }
            KeyCode::Char('q') => {
                self.should_quit = true;
            }
            _ => {}
        }
        Ok(())
    }

    fn open_seed_picker(&mut self) {
        let is_empty = self
            .session
            .garden()
            .plots()
            .get(self.selected_plot)
            .is_some_and(tg_core::model::Plot::is_empty);
        if !is_empty {
            self.status_message = format!(
                "Plot {} already holds a plant — select an empty plot to plant.",
                self.selected_plot + 1
            );
            return;
        }
        if self.available_seeds().is_empty() {
            self.status_message =
                "Seed pouch is empty — harvest a blooming plant or compost to gather seeds."
                    .to_string();
            return;
        }
        self.overlay = Some(Overlay::SeedPicker { cursor: 0 });
    }

    fn try_harvest_selected(&mut self, clock: &dyn Clock) -> Result<(), StoreError> {
        match self.session.harvest(self.selected_plot, clock.now()) {
            Ok(outcome) => {
                let drops: Vec<&str> = outcome
                    .dropped_seeds
                    .iter()
                    .map(|(_, name)| name.as_str())
                    .collect();
                self.status_message = format!(
                    "Harvested {}! Gathered seeds: {}.",
                    outcome.harvested_display_name,
                    drops.join(", ")
                );
            }
            Err(ActionError::Store(err)) => return Err(err),
            Err(err) => {
                self.status_message = err.to_string();
            }
        }
        Ok(())
    }

    fn navigate_plot(&mut self, code: KeyCode) -> bool {
        let count = self.session.garden().plots().len();
        if count == 0 {
            return false;
        }
        let cols = grid_columns_for(count);

        match code {
            KeyCode::Left => {
                self.selected_plot = if self.selected_plot == 0 {
                    count - 1
                } else {
                    self.selected_plot - 1
                };
                true
            }
            KeyCode::Right | KeyCode::Tab => {
                self.selected_plot = (self.selected_plot + 1) % count;
                true
            }
            KeyCode::BackTab => {
                self.selected_plot = (self.selected_plot + count - 1) % count;
                true
            }
            KeyCode::Up => {
                if self.selected_plot >= cols {
                    self.selected_plot -= cols;
                }
                true
            }
            KeyCode::Down => {
                if self.selected_plot + cols < count {
                    self.selected_plot += cols;
                }
                true
            }
            KeyCode::Char(ch @ '1'..='9') => {
                let idx = (ch as usize) - ('1' as usize);
                if idx < count {
                    self.selected_plot = idx;
                }
                true
            }
            _ => false,
        }
    }
}

const fn grid_columns_for(plot_count: usize) -> usize {
    if plot_count <= 1 {
        1
    } else if plot_count <= 4 {
        2
    } else if plot_count <= 9 {
        3
    } else {
        4
    }
}
