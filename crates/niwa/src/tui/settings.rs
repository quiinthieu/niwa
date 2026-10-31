//! Interactive TUI Settings screen model covering every `PLAN.md` §6 / [`AppConfig::ALL_KEYS`]
//! key and gating opt-in signal activation behind [`tg_signals::consent::ConsentCatalog`]
//! (`PLAN.md` §5, §6 & §10 T15).

use tg_signals::SignalKind;
use tg_store::config::{AppConfig, MAX_FPS, MIN_FPS};

/// Canonical settings rows displayed in the TUI Settings modal (`1:1` with [`AppConfig::ALL_KEYS`]).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum SettingsField {
    Sprites,
    Color,
    Fps,
    SeasonMode,
    Hemisphere,
    DormantAfterDays,
    PromptStyle,
    MenubarShowTitle,
    SignalsPaused,
    SignalActivityEnabled,
    SignalActivityConsentedAt,
    SignalGitEnabled,
    SignalGitConsentedAt,
    SignalShellEnabled,
    SignalShellConsentedAt,
    GitRepos,
    GitAuthorEmail,
}

impl SettingsField {
    /// Complete ordered list of settings rows (`17` keys matching [`AppConfig::ALL_KEYS`]).
    pub const ALL: [Self; 17] = [
        Self::Sprites,
        Self::Color,
        Self::Fps,
        Self::SeasonMode,
        Self::Hemisphere,
        Self::DormantAfterDays,
        Self::PromptStyle,
        Self::MenubarShowTitle,
        Self::SignalsPaused,
        Self::SignalActivityEnabled,
        Self::SignalActivityConsentedAt,
        Self::SignalGitEnabled,
        Self::SignalGitConsentedAt,
        Self::SignalShellEnabled,
        Self::SignalShellConsentedAt,
        Self::GitRepos,
        Self::GitAuthorEmail,
    ];

    /// Exact `config.toml` dot-key passed to `niwa config set <key> <value>`.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Sprites => "sprites",
            Self::Color => "color",
            Self::Fps => "fps",
            Self::SeasonMode => "season_mode",
            Self::Hemisphere => "hemisphere",
            Self::DormantAfterDays => "dormant_after_days",
            Self::PromptStyle => "prompt.style",
            Self::MenubarShowTitle => "menubar.show_title",
            Self::SignalsPaused => "signals.paused",
            Self::SignalActivityEnabled => "signals.activity.enabled",
            Self::SignalActivityConsentedAt => "signals.activity.consented_at",
            Self::SignalGitEnabled => "signals.git.enabled",
            Self::SignalGitConsentedAt => "signals.git.consented_at",
            Self::SignalShellEnabled => "signals.shell.enabled",
            Self::SignalShellConsentedAt => "signals.shell.consented_at",
            Self::GitRepos => "git.repos",
            Self::GitAuthorEmail => "git.author_email",
        }
    }

    /// Human-friendly row label displayed alongside the canonical dot-key.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Sprites => "Sprite set",
            Self::Color => "Color tier",
            Self::Fps => "Animation cap (fps)",
            Self::SeasonMode => "Season mode",
            Self::Hemisphere => "Hemisphere",
            Self::DormantAfterDays => "Dormant after (days)",
            Self::PromptStyle => "Prompt glyph style",
            Self::MenubarShowTitle => "Menu bar glyph title",
            Self::SignalsPaused => "Pause all signals",
            Self::SignalActivityEnabled => "Active time signal",
            Self::SignalActivityConsentedAt => "Active time consent ts",
            Self::SignalGitEnabled => "Git commits signal",
            Self::SignalGitConsentedAt => "Git commits consent ts",
            Self::SignalShellEnabled => "Shell commands signal",
            Self::SignalShellConsentedAt => "Shell commands consent ts",
            Self::GitRepos => "Git repository paths",
            Self::GitAuthorEmail => "Git author email",
        }
    }

    /// Context hint shown in the Settings footer when this row is highlighted.
    #[must_use]
    pub const fn hint(self) -> &'static str {
        match self {
            Self::Sprites => "[Left/Right/Enter] Toggle unicode or 7-bit ascii sprites",
            Self::Color => "[Left/Right/Enter] Cycle auto, truecolor, 256, 16, or none",
            Self::Fps => "[Left/Right] Adjust 1..=4 fps active cap (0 fps when idle)",
            Self::SeasonMode => "[Left/Right/Enter] Toggle real calendar vs garden season cycle",
            Self::Hemisphere => "[Left/Right/Enter] Toggle northern vs southern calendar seasons",
            Self::DormantAfterDays => {
                "[Left/Right] -1/+1 day | [Enter] Type positive days before resting"
            }
            Self::PromptStyle => "[Left/Right/Enter] Cycle glyph, emoji, or none for `niwa prompt`",
            Self::MenubarShowTitle => {
                "[Left/Right/Enter] Show or hide compact stage glyph in macOS menu bar"
            }
            Self::SignalsPaused => {
                "[Left/Right/Enter] Pause or resume opt-in signals while keeping consent"
            }
            Self::SignalActivityEnabled => {
                "[Enter/Space] Enable (shows local storage consent) or disable active time"
            }
            Self::SignalActivityConsentedAt => {
                "[Enter] Edit Unix timestamp or 'none' | [x/Del] Clear consent timestamp"
            }
            Self::SignalGitEnabled => {
                "[Enter/Space] Enable (shows local storage consent) or disable git commits"
            }
            Self::SignalGitConsentedAt => {
                "[Enter] Edit Unix timestamp or 'none' | [x/Del] Clear consent timestamp"
            }
            Self::SignalShellEnabled => {
                "[Enter/Space] Enable (shows local storage consent) or disable shell commands"
            }
            Self::SignalShellConsentedAt => {
                "[Enter] Edit Unix timestamp or 'none' | [x/Del] Clear consent timestamp"
            }
            Self::GitRepos => {
                "[Enter] Edit comma-separated local repo paths | [x/Del] Clear ('none')"
            }
            Self::GitAuthorEmail => {
                "[Enter] Edit git author email (gardener@example.com) | [x/Del] Clear"
            }
        }
    }

    /// Returns `Some(kind)` if enabling this field must present the shared signal consent modal.
    #[must_use]
    pub const fn signal_consent_kind(self) -> Option<SignalKind> {
        match self {
            Self::SignalActivityEnabled => Some(SignalKind::Activity),
            Self::SignalGitEnabled => Some(SignalKind::Git),
            Self::SignalShellEnabled => Some(SignalKind::Shell),
            _ => None,
        }
    }

    /// Whether pressing `Enter` opens an inline text buffer for this setting.
    #[must_use]
    pub const fn supports_inline_edit(self) -> bool {
        matches!(
            self,
            Self::DormantAfterDays
                | Self::SignalActivityConsentedAt
                | Self::SignalGitConsentedAt
                | Self::SignalShellConsentedAt
                | Self::GitRepos
                | Self::GitAuthorEmail
        )
    }

    /// Whether pressing `x` / `Delete` / `Backspace` in browse mode clears this setting to `"none"`.
    #[must_use]
    pub const fn supports_clear_to_none(self) -> bool {
        matches!(
            self,
            Self::SignalActivityConsentedAt
                | Self::SignalGitConsentedAt
                | Self::SignalShellConsentedAt
                | Self::GitRepos
                | Self::GitAuthorEmail
        )
    }

    /// Formats the current `AppConfig` value for display in the Settings list.
    #[must_use]
    pub fn display_value(self, config: &AppConfig) -> String {
        let raw = config.get_key(self.key()).unwrap_or_default();
        if raw.is_empty() {
            "none".to_string()
        } else {
            raw
        }
    }

    /// Initial buffer pre-populated when entering inline edit mode (`""` when unset/`"none"`).
    #[must_use]
    pub fn initial_edit_buffer(self, config: &AppConfig) -> String {
        let raw = config.get_key(self.key()).unwrap_or_default();
        if raw == "none" {
            String::new()
        } else {
            raw
        }
    }

    /// Computes the next value when pressing `Left` (`forward = false`) or `Right` (`forward = true`).
    #[must_use]
    pub fn step_directional(self, config: &AppConfig, forward: bool) -> Option<String> {
        match self {
            Self::Sprites => Some(cycle_options(
                &["unicode", "ascii"],
                config.sprites.as_str(),
                forward,
            )),
            Self::Color => Some(cycle_options(
                &["auto", "truecolor", "256", "16", "none"],
                config.color.as_str(),
                forward,
            )),
            Self::Fps => {
                let next = if forward {
                    (config.fps + 1).min(MAX_FPS)
                } else {
                    config.fps.saturating_sub(1).max(MIN_FPS)
                };
                Some(next.to_string())
            }
            Self::SeasonMode => {
                let current = config.get_key(self.key()).unwrap_or_default();
                Some(cycle_options(&["calendar", "garden"], &current, forward))
            }
            Self::Hemisphere => {
                let current = config.get_key(self.key()).unwrap_or_default();
                Some(cycle_options(&["north", "south"], &current, forward))
            }
            Self::DormantAfterDays => {
                let next = if forward {
                    config.dormant_after_days.saturating_add(1)
                } else {
                    config.dormant_after_days.saturating_sub(1).max(1)
                };
                Some(next.to_string())
            }
            Self::PromptStyle => Some(cycle_options(
                &["glyph", "emoji", "none"],
                config.prompt.style.as_str(),
                forward,
            )),
            Self::MenubarShowTitle => Some((!config.menubar.show_title).to_string()),
            Self::SignalsPaused => Some((!config.signals.paused).to_string()),
            _ => None,
        }
    }

    /// Computes the next value when pressing `Enter` or `Space` on a non-text, non-signal-consent row.
    #[must_use]
    pub fn cycle_on_activate(self, config: &AppConfig) -> Option<String> {
        match self {
            Self::Fps => {
                let next = if config.fps >= MAX_FPS {
                    MIN_FPS
                } else {
                    config.fps + 1
                };
                Some(next.to_string())
            }
            _ => self.step_directional(config, true),
        }
    }
}

fn cycle_options(options: &[&str], current: &str, forward: bool) -> String {
    let idx = options
        .iter()
        .position(|&candidate| candidate == current)
        .unwrap_or(0);
    let next_idx = if forward {
        (idx + 1) % options.len()
    } else {
        (idx + options.len() - 1) % options.len()
    };
    options[next_idx].to_string()
}

/// Mutable UI state of the open Settings overlay.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SettingsState {
    pub cursor: usize,
    pub edit_buffer: Option<String>,
    pub error_message: Option<String>,
}

impl SettingsState {
    #[must_use]
    pub fn selected_field(&self) -> SettingsField {
        let clamped = self.cursor.min(SettingsField::ALL.len() - 1);
        SettingsField::ALL[clamped]
    }
}
