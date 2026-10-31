//! Ultra-low-latency `niwa prompt [--style glyph|emoji|none]` snapshot consumer and
//! `tmux` / Starship snippet emitters (`PLAN.md` §3.4, §4.3, §6, §7 & §10 T13).
//!
//! When `snapshot.json` exists and is `< 60 s` old ([`SNAPSHOT_MAX_AGE_SECS`]), `niwa prompt`
//! reads only `config.toml` and `snapshot.json`—never acquiring `garden.lock`, never loading
//! `garden.json` or embedded simulation catalogs, and never running `catch_up`.

use crate::runtime::GardenSession;
use clap::Args;
use std::io::{self, Write};
use std::process::ExitCode;
use std::str::FromStr;
use tg_core::clock::Clock;
use tg_core::model::Stage;
use tg_core::product::{CLI_BIN, DISPLAY_NAME};
use tg_core::species::SpriteSet;
use tg_render::glyph::{prompt_emoji_glyph, stage_from_glyph, stage_glyph};
use tg_store::clock::SystemClock;
use tg_store::config::{ConfigStore, PromptStyle};
use tg_store::paths::NiwaPaths;
use tg_store::snapshot::{read_snapshot, Snapshot, SNAPSHOT_MAX_AGE_SECS};

#[derive(Args, Clone, Debug, Default, PartialEq, Eq)]
pub struct PromptArgs {
    /// Override prompt display style (`glyph`, `emoji`, or `none`; defaults to `config.toml` `prompt.style`).
    #[arg(long, value_name = "glyph|emoji|none")]
    pub style: Option<PromptStyle>,

    /// Print a ready-to-paste `~/.tmux.conf` status-bar snippet and exit.
    #[arg(long, conflicts_with = "starship")]
    pub tmux: bool,

    /// Print a ready-to-paste `~/.config/starship.toml` custom-module snippet and exit.
    #[arg(long, conflicts_with = "tmux")]
    pub starship: bool,
}

/// Evaluates `niwa prompt` against `paths` and `clock`, writing to `writer`.
pub fn run_prompt_with_writer(
    paths: &NiwaPaths,
    clock: &dyn Clock,
    args: &PromptArgs,
    writer: &mut dyn Write,
) -> anyhow::Result<()> {
    if args.tmux {
        write!(writer, "{}", render_tmux_snippet(args.style))?;
        return Ok(());
    }
    if args.starship {
        write!(writer, "{}", render_starship_snippet(args.style))?;
        return Ok(());
    }

    // Immediate zero-I/O short-circuit when `--style none` is explicitly requested on the CLI.
    if args.style == Some(PromptStyle::None) {
        return Ok(());
    }

    let (style, sprites) = if args.style == Some(PromptStyle::Emoji) {
        // `--style emoji` inverts both Unicode and ASCII stage glyphs identically,
        // avoiding even reading `config.toml` on the fresh-snapshot fast path.
        (PromptStyle::Emoji, SpriteSet::Unicode)
    } else {
        let config = ConfigStore::new(paths.clone()).load()?;
        let resolved_style = args.style.unwrap_or(config.prompt.style);
        if resolved_style == PromptStyle::None {
            return Ok(());
        }
        (resolved_style, config.sprites)
    };

    let now = clock.now();
    let existing = read_snapshot(paths)?;

    // Fast path (`PLAN.md` §3.4 & §10 T13): when `snapshot.json` is `< 60 s` old, read it
    // directly with zero locking, zero `garden.json` I/O, and zero simulation.
    if let Some(ref snap) = existing {
        if snap.is_fresh(now, SNAPSHOT_MAX_AGE_SECS) {
            if let Some(token) = format_snapshot_for_prompt(snap, sprites, style) {
                writeln!(writer, "{token}")?;
            }
            return Ok(());
        }
    }

    // Missing or `>= 60 s` stale snapshot: attempt a non-blocking refresh (`GardenLock::try_acquire`).
    // If another process (such as an open interactive TUI) holds `garden.lock`, never block the prompt.
    if let Some(refreshed) = GardenSession::try_refresh_snapshot_for_prompt(paths.clone(), clock)? {
        if let Some(token) = format_snapshot_for_prompt(&refreshed, sprites, style) {
            writeln!(writer, "{token}")?;
        }
        return Ok(());
    }

    if let Some(ref stale) = existing {
        if let Some(token) = format_snapshot_for_prompt(stale, sprites, style) {
            writeln!(writer, "{token}")?;
        }
    } else if let Some(token) = format_stage_for_prompt(None, sprites, style) {
        writeln!(writer, "{token}")?;
    }

    Ok(())
}

/// Renders the prompt token for `snapshot`, honouring `sprites` (`unicode` vs `ascii`) and `style`.
#[must_use]
pub fn format_snapshot_for_prompt(
    snapshot: &Snapshot,
    sprites: SpriteSet,
    style: PromptStyle,
) -> Option<String> {
    match style {
        PromptStyle::None => None,
        PromptStyle::Glyph => match stage_from_glyph(&snapshot.glyph) {
            Some(stage) => Some(stage_glyph(stage, sprites).to_string()),
            None => Some(snapshot.glyph.clone()),
        },
        PromptStyle::Emoji => {
            let stage = match stage_from_glyph(&snapshot.glyph) {
                Some(stage) => stage,
                None if snapshot.blooming_count > 0 => Some(Stage::Blooming),
                None if snapshot.planted_count == 0 => None,
                None => Some(Stage::Sprout),
            };
            Some(prompt_emoji_glyph(stage).to_string())
        }
    }
}

fn format_stage_for_prompt(
    stage: Option<Stage>,
    sprites: SpriteSet,
    style: PromptStyle,
) -> Option<&'static str> {
    match style {
        PromptStyle::None => None,
        PromptStyle::Glyph => Some(stage_glyph(stage, sprites)),
        PromptStyle::Emoji => Some(prompt_emoji_glyph(stage)),
    }
}

/// Renders a ready-to-paste `~/.tmux.conf` status-bar snippet (`PLAN.md` §7 & §10 T13).
#[must_use]
pub fn render_tmux_snippet(style: Option<PromptStyle>) -> String {
    let cmd = prompt_command_string(style);
    format!(
        "# {DISPLAY_NAME} garden status glyph (60 s interval matches {DISPLAY_NAME}'s 60 s garden tick budget)\n\
         set -g status-interval 60\n\
         set -ag status-right \" #({cmd})\"\n"
    )
}

/// Renders a ready-to-paste `~/.config/starship.toml` custom-module block (`PLAN.md` §10 T13).
#[must_use]
pub fn render_starship_snippet(style: Option<PromptStyle>) -> String {
    let cmd = prompt_command_string(style);
    format!(
        "# {DISPLAY_NAME} idle garden glyph (reads fresh <60 s snapshot.json in a few milliseconds)\n\
         [custom.{CLI_BIN}]\n\
         command = \"{cmd}\"\n\
         when = true\n\
         format = \"[$output ]($style)\"\n\
         style = \"green\"\n\
         ignore_timeout = true\n\
         description = \"{DISPLAY_NAME} terminal garden stage indicator\"\n"
    )
}

fn prompt_command_string(style: Option<PromptStyle>) -> String {
    match style {
        Some(explicit) => format!("{CLI_BIN} prompt --style {explicit}"),
        None => format!("{CLI_BIN} prompt"),
    }
}

/// Direct `std::env::args_os()` fast path bypassing full `clap` command-tree construction
/// when invoked as `niwa prompt [--style glyph|emoji|none]`.
///
/// Returns `None` for any non-`prompt` command or when `--help`, `--tmux`, `--starship`, or
/// invalid/unknown flags are passed so `clap` handles standard help, snippets, and usage errors.
#[must_use]
pub fn try_run_fast_prompt_from_env() -> Option<ExitCode> {
    let mut raw_args = std::env::args_os();
    let _bin = raw_args.next()?;
    let first = raw_args.next()?;

    let rest_after_subcmd: Vec<String> = if first == "prompt" {
        raw_args
            .map(|os| os.into_string().ok())
            .collect::<Option<Vec<_>>>()?
    } else if first == "--no-color" {
        let sub = raw_args.next()?;
        if sub != "prompt" {
            return None;
        }
        raw_args
            .map(|os| os.into_string().ok())
            .collect::<Option<Vec<_>>>()?
    } else {
        return None;
    };

    let parsed = parse_fast_prompt_tokens(&rest_after_subcmd)?;
    let result = NiwaPaths::resolve()
        .map_err(anyhow::Error::from)
        .and_then(|paths| {
            let mut stdout = io::stdout().lock();
            run_prompt_with_writer(&paths, &SystemClock, &parsed, &mut stdout)
        });

    Some(match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    })
}

fn parse_fast_prompt_tokens(tokens: &[String]) -> Option<PromptArgs> {
    let mut style: Option<PromptStyle> = None;
    let mut idx = 0_usize;

    while idx < tokens.len() {
        let tok = tokens[idx].as_str();
        match tok {
            "--no-color" => {
                idx += 1;
            }
            "--style" => {
                let raw = tokens.get(idx + 1)?;
                style = Some(PromptStyle::from_str(raw).ok()?);
                idx += 2;
            }
            _ if tok.starts_with("--style=") => {
                let raw = tok.strip_prefix("--style=")?;
                style = Some(PromptStyle::from_str(raw).ok()?);
                idx += 1;
            }
            _ => return None,
        }
    }

    Some(PromptArgs {
        style,
        tmux: false,
        starship: false,
    })
}
