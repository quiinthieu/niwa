//! Terminal color tiers, `NO_COLOR` / `--no-color` resolution, and ANSI SGR styling (PLAN §4.3).

use serde::{Deserialize, Serialize};
use std::fmt::Write;

/// Concrete resolved color output tier for rendering frames.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorTier {
    #[default]
    NoColor,
    Ansi16,
    Ansi256,
    Truecolor,
}

impl ColorTier {
    pub const ALL: [Self; 4] = [Self::NoColor, Self::Ansi16, Self::Ansi256, Self::Truecolor];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoColor => "no_color",
            Self::Ansi16 => "ansi16",
            Self::Ansi256 => "ansi256",
            Self::Truecolor => "truecolor",
        }
    }

    #[must_use]
    pub const fn is_colored(self) -> bool {
        !matches!(self, Self::NoColor)
    }
}

/// User-configured color preference (`auto`, `truecolor`, `256`, `16`, or `none`; PLAN §6).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorPreference {
    #[default]
    Auto,
    Truecolor,
    #[serde(rename = "256")]
    Ansi256,
    #[serde(rename = "16")]
    Ansi16,
    None,
}

/// Pure terminal environment snapshot used to resolve [`ColorTier`] without performing I/O.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct ColorEnv<'a> {
    pub no_color_flag: bool,
    pub no_color_env: Option<&'a str>,
    pub colorterm: Option<&'a str>,
    pub term: Option<&'a str>,
}

/// Resolves the concrete [`ColorTier`] honouring `--no-color`, `NO_COLOR`, and terminal hints (PLAN §4.3).
#[must_use]
pub fn detect_color_tier(preference: ColorPreference, env: &ColorEnv<'_>) -> ColorTier {
    if env.no_color_flag {
        return ColorTier::NoColor;
    }
    if env.no_color_env.is_some_and(|val| !val.is_empty()) {
        return ColorTier::NoColor;
    }

    match preference {
        ColorPreference::None => ColorTier::NoColor,
        ColorPreference::Ansi16 => ColorTier::Ansi16,
        ColorPreference::Ansi256 => ColorTier::Ansi256,
        ColorPreference::Truecolor => ColorTier::Truecolor,
        ColorPreference::Auto => detect_auto_tier(env),
    }
}

fn detect_auto_tier(env: &ColorEnv<'_>) -> ColorTier {
    if env
        .term
        .is_some_and(|term| term.eq_ignore_ascii_case("dumb"))
    {
        return ColorTier::NoColor;
    }

    if let Some(ct) = env.colorterm {
        let lower = ct.to_ascii_lowercase();
        if lower.contains("truecolor") || lower.contains("24bit") {
            return ColorTier::Truecolor;
        }
    }

    if let Some(term) = env.term {
        if term.is_empty() {
            return ColorTier::NoColor;
        }
        let lower = term.to_ascii_lowercase();
        if lower.contains("truecolor")
            || lower.contains("24bit")
            || lower.contains("kitty")
            || lower.contains("ghostty")
            || lower.contains("wezterm")
            || lower.contains("alacritty")
        {
            return ColorTier::Truecolor;
        }
        if lower.contains("256color") {
            return ColorTier::Ansi256;
        }
        return ColorTier::Ansi16;
    }

    ColorTier::NoColor
}

/// Standard 16-color ANSI foreground SGR codes (`30..=37` and `90..=97`).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Ansi16Color {
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    BrightBlack,
    BrightRed,
    BrightGreen,
    BrightYellow,
    BrightBlue,
    BrightMagenta,
    BrightCyan,
    BrightWhite,
}

impl Ansi16Color {
    #[must_use]
    pub const fn fg_sgr_code(self) -> u8 {
        match self {
            Self::Black => 30,
            Self::Red => 31,
            Self::Green => 32,
            Self::Yellow => 33,
            Self::Blue => 34,
            Self::Magenta => 35,
            Self::Cyan => 36,
            Self::White => 37,
            Self::BrightBlack => 90,
            Self::BrightRed => 91,
            Self::BrightGreen => 92,
            Self::BrightYellow => 93,
            Self::BrightBlue => 94,
            Self::BrightMagenta => 95,
            Self::BrightCyan => 96,
            Self::BrightWhite => 97,
        }
    }
}

/// Multi-tier foreground style carrying coordinated Truecolor, 256-color, and 16-color encodings.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct StyleSpec {
    pub rgb: (u8, u8, u8),
    pub ansi256: u8,
    pub ansi16: Ansi16Color,
    pub bold: bool,
    pub dim: bool,
}

impl StyleSpec {
    #[must_use]
    pub const fn new(rgb: (u8, u8, u8), ansi256: u8, ansi16: Ansi16Color) -> Self {
        Self {
            rgb,
            ansi256,
            ansi16,
            bold: false,
            dim: false,
        }
    }

    #[must_use]
    pub const fn with_bold(mut self) -> Self {
        self.bold = true;
        self
    }

    #[must_use]
    pub const fn with_dim(mut self) -> Self {
        self.dim = true;
        self
    }

    /// Writes the opening SGR sequence for `tier` into `buf` (does nothing for [`ColorTier::NoColor`]).
    pub fn write_sgr_open(self, tier: ColorTier, buf: &mut String) {
        if tier == ColorTier::NoColor {
            return;
        }
        buf.push_str("\x1b[");
        let mut need_sep = false;
        if self.bold {
            buf.push('1');
            need_sep = true;
        }
        if self.dim {
            if need_sep {
                buf.push(';');
            }
            buf.push('2');
            need_sep = true;
        }
        if need_sep {
            buf.push(';');
        }
        match tier {
            ColorTier::NoColor => unreachable!(),
            ColorTier::Ansi16 => {
                let _ = write!(buf, "{}m", self.ansi16.fg_sgr_code());
            }
            ColorTier::Ansi256 => {
                let _ = write!(buf, "38;5;{}m", self.ansi256);
            }
            ColorTier::Truecolor => {
                let (r, g, b) = self.rgb;
                let _ = write!(buf, "38;2;{r};{g};{b}m");
            }
        }
    }
}
