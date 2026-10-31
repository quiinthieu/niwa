//! `config.toml` schema, validation, dot-key `get`/`set`, and atomic persistence (PLAN §6).

use crate::atomic::atomic_write_bytes;
use crate::error::StoreError;
use crate::paths::NiwaPaths;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::PathBuf;
use tg_core::clock::Timestamp;
use tg_core::season::{Hemisphere, SeasonMode};

pub const DEFAULT_FPS: u8 = 4;
pub const MIN_FPS: u8 = 1;
pub const MAX_FPS: u8 = 4;
pub const DEFAULT_DORMANT_AFTER_DAYS: u32 = 7;

pub use tg_core::species::SpriteSet;

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorSetting {
    #[default]
    Auto,
    Truecolor,
    #[serde(rename = "256")]
    Ansi256,
    #[serde(rename = "16")]
    Ansi16,
    None,
}

impl ColorSetting {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Truecolor => "truecolor",
            Self::Ansi256 => "256",
            Self::Ansi16 => "16",
            Self::None => "none",
        }
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptStyle {
    #[default]
    Glyph,
    Emoji,
    None,
}

impl PromptStyle {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Glyph => "glyph",
            Self::Emoji => "emoji",
            Self::None => "none",
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum SignalKind {
    Activity,
    Git,
    Shell,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignalConsent {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consented_at: Option<Timestamp>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignalsConfig {
    #[serde(default)]
    pub activity: SignalConsent,
    #[serde(default)]
    pub git: SignalConsent,
    #[serde(default)]
    pub shell: SignalConsent,
}

impl SignalsConfig {
    pub fn consent_mut(&mut self, signal: SignalKind) -> &mut SignalConsent {
        match signal {
            SignalKind::Activity => &mut self.activity,
            SignalKind::Git => &mut self.git,
            SignalKind::Shell => &mut self.shell,
        }
    }

    #[must_use]
    pub const fn consent(&self, signal: SignalKind) -> &SignalConsent {
        match signal {
            SignalKind::Activity => &self.activity,
            SignalKind::Git => &self.git,
            SignalKind::Shell => &self.shell,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitConfig {
    #[serde(default)]
    pub repos: Vec<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_email: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptConfig {
    #[serde(default)]
    pub style: PromptStyle,
}

const fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MenubarConfig {
    #[serde(default = "default_true")]
    pub show_title: bool,
}

impl Default for MenubarConfig {
    fn default() -> Self {
        Self { show_title: true }
    }
}

const fn default_fps() -> u8 {
    DEFAULT_FPS
}

const fn default_dormant_after_days() -> u32 {
    DEFAULT_DORMANT_AFTER_DAYS
}

/// Complete `config.toml` representation covering every `PLAN.md` §6 setting.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub sprites: SpriteSet,
    #[serde(default)]
    pub color: ColorSetting,
    #[serde(default = "default_fps")]
    pub fps: u8,
    #[serde(default)]
    pub season_mode: SeasonMode,
    #[serde(default)]
    pub hemisphere: Hemisphere,
    #[serde(default = "default_dormant_after_days")]
    pub dormant_after_days: u32,
    #[serde(default)]
    pub signals: SignalsConfig,
    #[serde(default)]
    pub git: GitConfig,
    #[serde(default)]
    pub prompt: PromptConfig,
    #[serde(default)]
    pub menubar: MenubarConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            sprites: SpriteSet::default(),
            color: ColorSetting::default(),
            fps: DEFAULT_FPS,
            season_mode: SeasonMode::default(),
            hemisphere: Hemisphere::default(),
            dormant_after_days: DEFAULT_DORMANT_AFTER_DAYS,
            signals: SignalsConfig::default(),
            git: GitConfig::default(),
            prompt: PromptConfig::default(),
            menubar: MenubarConfig::default(),
        }
    }
}

impl AppConfig {
    pub fn validate(&self) -> Result<(), StoreError> {
        if !(MIN_FPS..=MAX_FPS).contains(&self.fps) {
            return Err(StoreError::InvalidConfigValue {
                key: "fps".to_string(),
                value: self.fps.to_string(),
                reason: format!("fps must be between {MIN_FPS} and {MAX_FPS} (PLAN §7)"),
            });
        }
        if self.dormant_after_days == 0 {
            return Err(StoreError::InvalidConfigValue {
                key: "dormant_after_days".to_string(),
                value: "0".to_string(),
                reason: "dormant_after_days must be at least 1".to_string(),
            });
        }
        Ok(())
    }

    pub fn enable_signal(&mut self, signal: SignalKind, now: Timestamp) {
        let entry = self.signals.consent_mut(signal);
        entry.enabled = true;
        entry.consented_at = Some(now);
    }

    pub fn disable_signal(&mut self, signal: SignalKind, clear_consent: bool) {
        let entry = self.signals.consent_mut(signal);
        entry.enabled = false;
        if clear_consent {
            entry.consented_at = None;
        }
    }

    pub fn get_key(&self, key: &str) -> Result<String, StoreError> {
        match key {
            "sprites" => Ok(self.sprites.as_str().to_string()),
            "color" => Ok(self.color.as_str().to_string()),
            "fps" => Ok(self.fps.to_string()),
            "season_mode" => Ok(match self.season_mode {
                SeasonMode::Calendar => "calendar",
                SeasonMode::Garden => "garden",
            }
            .to_string()),
            "hemisphere" => Ok(match self.hemisphere {
                Hemisphere::North => "north",
                Hemisphere::South => "south",
            }
            .to_string()),
            "dormant_after_days" => Ok(self.dormant_after_days.to_string()),
            "signals.activity" | "signals.activity.enabled" => {
                Ok(self.signals.activity.enabled.to_string())
            }
            "signals.activity.consented_at" => {
                Ok(format_timestamp(self.signals.activity.consented_at))
            }
            "signals.git" | "signals.git.enabled" => Ok(self.signals.git.enabled.to_string()),
            "signals.git.consented_at" => Ok(format_timestamp(self.signals.git.consented_at)),
            "signals.shell" | "signals.shell.enabled" => Ok(self.signals.shell.enabled.to_string()),
            "signals.shell.consented_at" => Ok(format_timestamp(self.signals.shell.consented_at)),
            "git.repos" => Ok(self
                .git
                .repos
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join(",")),
            "git.author_email" => Ok(self.git.author_email.clone().unwrap_or_default()),
            "prompt.style" => Ok(self.prompt.style.as_str().to_string()),
            "menubar.show_title" => Ok(self.menubar.show_title.to_string()),
            _ => Err(StoreError::InvalidConfigKey {
                key: key.to_string(),
            }),
        }
    }

    pub fn set_key(&mut self, key: &str, value: &str) -> Result<(), StoreError> {
        self.set_key_with_timestamp(key, value, None)
    }

    pub fn set_key_with_timestamp(
        &mut self,
        key: &str,
        value: &str,
        now: Option<Timestamp>,
    ) -> Result<(), StoreError> {
        match key {
            "sprites" => {
                self.sprites = match value {
                    "unicode" => SpriteSet::Unicode,
                    "ascii" => SpriteSet::Ascii,
                    _ => return Err(invalid_val(key, value, "expected 'unicode' or 'ascii'")),
                };
            }
            "color" => {
                self.color = match value {
                    "auto" => ColorSetting::Auto,
                    "truecolor" => ColorSetting::Truecolor,
                    "256" => ColorSetting::Ansi256,
                    "16" => ColorSetting::Ansi16,
                    "none" => ColorSetting::None,
                    _ => {
                        return Err(invalid_val(
                            key,
                            value,
                            "expected 'auto', 'truecolor', '256', '16', or 'none'",
                        ));
                    }
                };
            }
            "fps" => {
                let parsed: u8 = value.parse().map_err(|_| {
                    invalid_val(
                        key,
                        value,
                        &format!("expected integer between {MIN_FPS} and {MAX_FPS}"),
                    )
                })?;
                if !(MIN_FPS..=MAX_FPS).contains(&parsed) {
                    return Err(invalid_val(
                        key,
                        value,
                        &format!("fps must be between {MIN_FPS} and {MAX_FPS} (PLAN §7)"),
                    ));
                }
                self.fps = parsed;
            }
            "season_mode" => {
                self.season_mode = match value {
                    "calendar" => SeasonMode::Calendar,
                    "garden" => SeasonMode::Garden,
                    _ => return Err(invalid_val(key, value, "expected 'calendar' or 'garden'")),
                };
            }
            "hemisphere" => {
                self.hemisphere = match value {
                    "north" => Hemisphere::North,
                    "south" => Hemisphere::South,
                    _ => return Err(invalid_val(key, value, "expected 'north' or 'south'")),
                };
            }
            "dormant_after_days" => {
                let parsed: u32 = value
                    .parse()
                    .map_err(|_| invalid_val(key, value, "expected positive integer"))?;
                if parsed == 0 {
                    return Err(invalid_val(
                        key,
                        value,
                        "dormant_after_days must be at least 1",
                    ));
                }
                self.dormant_after_days = parsed;
            }
            "signals.activity" | "signals.activity.enabled" => {
                set_signal_enabled(&mut self.signals.activity, key, value, now)?;
            }
            "signals.activity.consented_at" => {
                self.signals.activity.consented_at = parse_optional_timestamp(key, value)?;
            }
            "signals.git" | "signals.git.enabled" => {
                set_signal_enabled(&mut self.signals.git, key, value, now)?;
            }
            "signals.git.consented_at" => {
                self.signals.git.consented_at = parse_optional_timestamp(key, value)?;
            }
            "signals.shell" | "signals.shell.enabled" => {
                set_signal_enabled(&mut self.signals.shell, key, value, now)?;
            }
            "signals.shell.consented_at" => {
                self.signals.shell.consented_at = parse_optional_timestamp(key, value)?;
            }
            "git.repos" => {
                self.git.repos = parse_repos(key, value)?;
            }
            "git.author_email" => {
                let trimmed = value.trim();
                self.git.author_email = (!trimmed.is_empty()).then(|| trimmed.to_string());
            }
            "prompt.style" => {
                self.prompt.style = match value {
                    "glyph" => PromptStyle::Glyph,
                    "emoji" => PromptStyle::Emoji,
                    "none" => PromptStyle::None,
                    _ => {
                        return Err(invalid_val(
                            key,
                            value,
                            "expected 'glyph', 'emoji', or 'none'",
                        ));
                    }
                };
            }
            "menubar.show_title" => {
                self.menubar.show_title = parse_bool(key, value)?;
            }
            _ => {
                return Err(StoreError::InvalidConfigKey {
                    key: key.to_string(),
                });
            }
        }
        Ok(())
    }
}

/// Reader and atomic writer for `config.toml`.
#[derive(Clone, Debug)]
pub struct ConfigStore {
    paths: NiwaPaths,
}

impl ConfigStore {
    #[must_use]
    pub fn new(paths: NiwaPaths) -> Self {
        Self { paths }
    }

    #[must_use]
    pub fn paths(&self) -> &NiwaPaths {
        &self.paths
    }

    pub fn load(&self) -> Result<AppConfig, StoreError> {
        let path = self.paths.config_file();
        let raw = match fs::read_to_string(&path) {
            Ok(content) => content,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                return Ok(AppConfig::default());
            }
            Err(source) => {
                return Err(StoreError::Io {
                    path,
                    action: "read config.toml",
                    source,
                });
            }
        };

        let config: AppConfig =
            toml::from_str(&raw).map_err(|source| StoreError::TomlParse { path, source })?;
        config.validate()?;
        Ok(config)
    }

    pub fn save(&self, config: &AppConfig) -> Result<(), StoreError> {
        config.validate()?;
        let serialized = toml::to_string_pretty(config)
            .map_err(|source| StoreError::TomlSerialize { source })?;
        atomic_write_bytes(&self.paths.config_file(), None, serialized.as_bytes(), None)
    }
}

fn invalid_val(key: &str, value: &str, reason: &str) -> StoreError {
    StoreError::InvalidConfigValue {
        key: key.to_string(),
        value: value.to_string(),
        reason: reason.to_string(),
    }
}

fn parse_bool(key: &str, value: &str) -> Result<bool, StoreError> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(invalid_val(key, value, "expected 'true' or 'false'")),
    }
}

fn set_signal_enabled(
    consent: &mut SignalConsent,
    key: &str,
    value: &str,
    now: Option<Timestamp>,
) -> Result<(), StoreError> {
    let enabled = parse_bool(key, value)?;
    consent.enabled = enabled;
    if enabled {
        if let Some(ts) = now {
            consent.consented_at = Some(ts);
        }
    }
    Ok(())
}

fn format_timestamp(ts: Option<Timestamp>) -> String {
    match ts {
        Some(val) => val.as_unix_secs().to_string(),
        None => "none".to_string(),
    }
}

fn parse_optional_timestamp(key: &str, value: &str) -> Result<Option<Timestamp>, StoreError> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed == "none" {
        return Ok(None);
    }
    let secs: i64 = trimmed
        .parse()
        .map_err(|_| invalid_val(key, value, "expected Unix timestamp integer or 'none'"))?;
    Ok(Some(Timestamp::from_unix_secs(secs)))
}

fn parse_repos(key: &str, value: &str) -> Result<Vec<PathBuf>, StoreError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    if trimmed.starts_with('[') {
        let items: Vec<String> = serde_json::from_str(trimmed)
            .map_err(|_| invalid_val(key, value, "invalid JSON array of repository paths"))?;
        return Ok(items
            .into_iter()
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
            .map(PathBuf::from)
            .collect());
    }

    Ok(trimmed
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(PathBuf::from)
        .collect())
}
