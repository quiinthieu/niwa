//! `clap` command-line interface and subcommand handlers for `niwa` (PLAN §3.4, §5, §6 & §10 T08).

use crate::runtime::{format_status_human, resolve_color_tier, GardenSession};
use crate::tui::ui::run_interactive_tui;
use clap::{Args, Parser, Subcommand};
use std::io::{self, BufRead, Write};
use tg_core::clock::Clock;
use tg_core::product;
use tg_core::species::SpriteSet;
use tg_render::Viewport;
use tg_store::clock::SystemClock;
use tg_store::config::{AppConfig, ConfigStore};
use tg_store::paths::NiwaPaths;
use tg_store::save::GardenStore;

const DEFAULT_PEEK_WIDTH: u16 = 80;
const DEFAULT_PEEK_HEIGHT: u16 = 24;

#[derive(Parser, Debug)]
#[command(
    name = product::CLI_BIN,
    bin_name = product::CLI_BIN,
    version = product::VERSION,
    about = "A cozy idle garden for your terminal and menu bar"
)]
pub struct Cli {
    /// Disable ANSI color sequences in output (also honours `NO_COLOR`).
    #[arg(long, global = true)]
    pub no_color: bool,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Render a single snapshot frame of your garden to stdout without opening the TUI.
    Peek(PeekArgs),
    /// Inspect garden status in human-readable form or versioned JSON (`--json`).
    Status(StatusArgs),
    /// Read or update `config.toml` settings.
    Config {
        #[command(subcommand)]
        action: ConfigCommand,
    },
    /// Delete persisted garden state (and `--all` to also remove `config.toml` and `events.jsonl`).
    Reset(ResetArgs),
}

#[derive(Args, Debug)]
pub struct PeekArgs {
    /// Override render viewport width in terminal columns (defaults to terminal size or 80).
    #[arg(long)]
    pub width: Option<u16>,

    /// Override render viewport height in terminal rows (defaults to terminal size or 24).
    #[arg(long)]
    pub height: Option<u16>,

    /// Render using the 7-bit ASCII sprite set regardless of `config.toml`.
    #[arg(long)]
    pub ascii: bool,
}

#[derive(Args, Debug)]
pub struct StatusArgs {
    /// Emit structured JSON (`schema_version = 1`).
    #[arg(long)]
    pub json: bool,
}

#[derive(Subcommand, Debug)]
pub enum ConfigCommand {
    /// Print the current value of a setting key (e.g. `fps`, `sprites`, `hemisphere`).
    Get { key: String },
    /// Update and validate a setting key in `config.toml`.
    Set { key: String, value: String },
    /// List all supported settings and their current values.
    List,
    /// Print the resolved path to `config.toml`.
    Path,
}

#[derive(Args, Debug)]
pub struct ResetArgs {
    /// Also delete `config.toml`, `events.jsonl`, and `garden.lock` (PLAN §5).
    #[arg(long)]
    pub all: bool,

    /// Skip the interactive `[y/N]` confirmation prompt.
    #[arg(short = 'y', long)]
    pub yes: bool,
}

/// Entrypoint executing a parsed [`Cli`] against the host environment and [`SystemClock`].
pub fn execute(cli: Cli) -> anyhow::Result<()> {
    let paths = NiwaPaths::resolve()?;
    let clock = SystemClock;
    execute_with(cli, paths, &clock)
}

/// Testable CLI executor accepting injected [`NiwaPaths`] and [`Clock`].
pub fn execute_with(cli: Cli, paths: NiwaPaths, clock: &dyn Clock) -> anyhow::Result<()> {
    match cli.command {
        None => {
            let session = GardenSession::open(paths, clock, None)?;
            emit_recovery_notice(&session);
            let color_tier = resolve_color_tier(session.config(), cli.no_color);
            run_interactive_tui(session, color_tier, clock)
        }
        Some(Commands::Peek(args)) => run_peek(paths, clock, cli.no_color, &args),
        Some(Commands::Status(args)) => run_status(paths, clock, &args),
        Some(Commands::Config { action }) => run_config(paths, clock, action),
        Some(Commands::Reset(args)) => run_reset(paths, &args),
    }
}

fn emit_recovery_notice(session: &GardenSession) {
    if let Some(notice) = session.recovery_notice() {
        eprintln!("warning: {notice}");
    }
}

fn run_peek(
    paths: NiwaPaths,
    clock: &dyn Clock,
    no_color_flag: bool,
    args: &PeekArgs,
) -> anyhow::Result<()> {
    let session = GardenSession::open(paths, clock, None)?;
    emit_recovery_notice(&session);

    let (detected_w, detected_h) = detect_terminal_size();
    let width = args.width.unwrap_or(detected_w).max(1);
    let height = args.height.unwrap_or(detected_h).max(1);
    let sprites_override = args.ascii.then_some(SpriteSet::Ascii);
    let color_tier = resolve_color_tier(session.config(), no_color_flag);

    let frame = session.render_frame(Viewport::new(width, height), sprites_override, color_tier);
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "{}", frame.render_for_tier(color_tier))?;
    if let Some(away) = session.last_away() {
        writeln!(stdout, "\n{}", away.render_text())?;
    }
    Ok(())
}

fn run_status(paths: NiwaPaths, clock: &dyn Clock, args: &StatusArgs) -> anyhow::Result<()> {
    let session = GardenSession::open(paths, clock, None)?;
    emit_recovery_notice(&session);

    let report = session.status_report();
    let mut stdout = io::stdout().lock();
    if args.json {
        let json = serde_json::to_string_pretty(&report)?;
        writeln!(stdout, "{json}")?;
    } else {
        writeln!(
            stdout,
            "{}",
            format_status_human(&report, session.catalog())
        )?;
    }
    Ok(())
}

fn run_config(paths: NiwaPaths, clock: &dyn Clock, action: ConfigCommand) -> anyhow::Result<()> {
    let store = ConfigStore::new(paths.clone());
    let mut stdout = io::stdout().lock();

    match action {
        ConfigCommand::Get { key } => {
            let config = store.load()?;
            let value = config.get_key(&key)?;
            writeln!(stdout, "{value}")?;
        }
        ConfigCommand::Set { key, value } => {
            let mut config = store.load()?;
            config.set_key_with_timestamp(&key, &value, Some(clock.now()))?;
            store.save(&config)?;
            let normalized = config.get_key(&key)?;
            writeln!(stdout, "{key} = {normalized}")?;
        }
        ConfigCommand::List => {
            let config = store.load()?;
            for key in AppConfig::ALL_KEYS {
                let value = config.get_key(key)?;
                writeln!(stdout, "{key} = {value}")?;
            }
        }
        ConfigCommand::Path => {
            writeln!(stdout, "{}", paths.config_file().display())?;
        }
    }
    Ok(())
}

fn run_reset(paths: NiwaPaths, args: &ResetArgs) -> anyhow::Result<()> {
    if !args.yes {
        let prompt = if args.all {
            "Reset Niwa garden save, snapshot, event spool, and config.toml? [y/N]: "
        } else {
            "Reset Niwa garden save and snapshot (keeping config.toml)? [y/N]: "
        };
        eprint!("{prompt}");
        io::stderr().flush()?;

        let mut answer = String::new();
        io::stdin().lock().read_line(&mut answer)?;
        let trimmed = answer.trim();
        if !trimmed.eq_ignore_ascii_case("y") && !trimmed.eq_ignore_ascii_case("yes") {
            println!("Reset cancelled.");
            return Ok(());
        }
    }

    let outcome = GardenStore::new(paths).reset(args.all)?;
    println!(
        "Reset complete ({} file(s) removed).",
        outcome.removed_files.len()
    );
    Ok(())
}

fn detect_terminal_size() -> (u16, u16) {
    match crossterm::terminal::size() {
        Ok((w, h)) if w > 0 && h > 0 => (w, h),
        _ => (DEFAULT_PEEK_WIDTH, DEFAULT_PEEK_HEIGHT),
    }
}
