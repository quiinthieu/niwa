//! `clap` command-line interface and subcommand handlers for `niwa` (PLAN §3.4, §5, §6, §10 T08–T10).

use crate::daemon::{
    run_daemon_install, run_daemon_loop, run_daemon_status, run_daemon_uninstall, ThreadSleeper,
};
use crate::hook::{run_hook_with_writer, HookArgs};
use crate::ingest::{run_ingest, IngestArgs};
use crate::runtime::{format_status_human, resolve_color_tier, GardenSession};
use crate::tui::ui::run_interactive_tui;
use clap::{Args, Parser, Subcommand};
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::str::FromStr;
use tg_core::clock::Clock;
use tg_core::product;
use tg_core::species::SpriteSet;
use tg_render::Viewport;
use tg_signals::activity::PlatformIdleSource;
use tg_signals::consent::{is_signal_enabled, ConsentCatalog};
use tg_signals::{EventSpool, SignalKind};
use tg_store::clock::SystemClock;
use tg_store::config::{AppConfig, ConfigStore};
use tg_store::daemon_units::DaemonUnitPaths;
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
    /// Inspect, enable (with explicit consent), or disable opt-in local work signals.
    Signals(SignalsArgs),
    /// Run the 60-second background daemon loop or install/uninstall per-user service units.
    Daemon(DaemonArgs),
    /// Print `zsh`, `bash`, or `fish` hook snippets or install/uninstall the marked rc block.
    Hook(HookArgs),
    /// Delete persisted garden state (and `--all` to also remove `config.toml` and `events.jsonl`).
    Reset(ResetArgs),
    /// Internal fast-path event appender invoked asynchronously by shell hooks.
    #[command(name = "_ingest", hide = true)]
    Ingest(IngestArgs),
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
pub struct SignalsArgs {
    #[command(subcommand)]
    pub action: Option<SignalsCommand>,
}

#[derive(Subcommand, Debug)]
pub enum SignalsCommand {
    /// List all work signals, their consent status, and queued spool counts.
    List,
    /// Show the exact storage disclosure for a signal and wait for `y` to opt in.
    Enable {
        /// Opt-in signal name (`activity`, `git`, or `shell`).
        signal: String,
        /// Accept the printed consent disclosure non-interactively without prompting stdin.
        #[arg(short = 'y', long)]
        yes: bool,
    },
    /// Stop collecting a signal (and with `--purge`, delete its queued events and consent record).
    Disable {
        /// Opt-in signal name (`activity`, `git`, or `shell`).
        signal: String,
        /// Delete queued events for this signal from `events.jsonl` and clear `consented_at`.
        #[arg(long)]
        purge: bool,
    },
}

#[derive(Args, Debug)]
#[command(args_conflicts_with_subcommands = true)]
pub struct DaemonArgs {
    /// Run a single daemon wake check and exit instead of looping every 60 seconds.
    #[arg(long)]
    pub once: bool,

    /// Stop after at most N wake iterations.
    #[arg(long, value_name = "N")]
    pub max_ticks: Option<u64>,

    #[command(subcommand)]
    pub action: Option<DaemonCommand>,
}

#[derive(Subcommand, Debug)]
pub enum DaemonCommand {
    /// Run the 60-second background daemon loop (default when no subcommand is given).
    Run {
        /// Run a single daemon wake check and exit instead of looping every 60 seconds.
        #[arg(long)]
        once: bool,

        /// Stop after at most N wake iterations.
        #[arg(long, value_name = "N")]
        max_ticks: Option<u64>,
    },
    /// Write the per-user launchd (macOS) or systemd (Linux) daemon unit file.
    Install {
        /// Override the `niwa` binary path embedded in the unit file (defaults to current executable).
        #[arg(long)]
        bin: Option<PathBuf>,
    },
    /// Remove the per-user launchd (macOS) or systemd (Linux) daemon unit file.
    Uninstall,
    /// Report the user unit path, installation state, and active signal wake status.
    Status,
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
        Some(Commands::Signals(args)) => run_signals(paths, clock, args),
        Some(Commands::Daemon(args)) => run_daemon(paths, clock, args),
        Some(Commands::Hook(args)) => {
            let mut stdout = io::stdout().lock();
            run_hook_with_writer(&paths, args, &mut stdout)
        }
        Some(Commands::Reset(args)) => run_reset(paths, &args),
        Some(Commands::Ingest(args)) => {
            let _ = run_ingest(&paths, clock, &args)?;
            Ok(())
        }
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

fn run_signals(paths: NiwaPaths, clock: &dyn Clock, args: SignalsArgs) -> anyhow::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = stdin.lock();
    let mut writer = stdout.lock();
    run_signals_with_io(paths, clock, args, &mut reader, &mut writer)
}

/// Testable `niwa signals` handler accepting injected [`BufRead`] and [`Write`] streams.
pub fn run_signals_with_io(
    paths: NiwaPaths,
    clock: &dyn Clock,
    args: SignalsArgs,
    reader: &mut dyn BufRead,
    writer: &mut dyn Write,
) -> anyhow::Result<()> {
    let config_store = ConfigStore::new(paths.clone());
    let spool = EventSpool::new(paths.clone());
    let catalog = ConsentCatalog::load_default();

    match args.action.unwrap_or(SignalsCommand::List) {
        SignalsCommand::List => {
            let config = config_store.load()?;
            let counts = spool.count_by_signal()?;
            let baseline = catalog.baseline();

            writeln!(
                writer,
                "time      always on  — {} (stores: {})",
                baseline.summary,
                baseline.stores.join("; ")
            )?;

            for kind in SignalKind::ALL {
                let disclosure = catalog.signal(kind);
                let consent = config.signals.consent(kind);
                let status = if is_signal_enabled(&config, kind) {
                    let ts = consent.consented_at.map_or(0, |t| t.as_unix_secs());
                    format!("enabled (consented_at = {ts})")
                } else {
                    "disabled".to_string()
                };
                let queued = counts.for_signal(kind);
                writeln!(
                    writer,
                    "{:<8}  {:<34}  queued: {:<4}  — {}",
                    kind.as_str(),
                    status,
                    queued,
                    disclosure.summary
                )?;
            }
        }
        SignalsCommand::Enable { signal, yes } => {
            let kind = SignalKind::from_str(&signal)?;
            let disclosure = catalog.signal(kind);

            write!(writer, "{}", disclosure.render_disclosure_text(kind))?;
            writer.flush()?;

            let accepted = if yes {
                writeln!(writer, "y")?;
                true
            } else {
                let mut answer = String::new();
                reader.read_line(&mut answer)?;
                let trimmed = answer.trim();
                trimmed.eq_ignore_ascii_case("y") || trimmed.eq_ignore_ascii_case("yes")
            };

            if !accepted {
                writeln!(
                    writer,
                    "Signal '{}' remains disabled (consent not given).",
                    kind.as_str()
                )?;
                return Ok(());
            }

            let now = clock.now();
            let mut config = config_store.load()?;
            config.enable_signal(kind, now);
            config_store.save(&config)?;
            writeln!(
                writer,
                "Enabled signal '{}' (consented_at = {}).",
                kind.as_str(),
                now.as_unix_secs()
            )?;
        }
        SignalsCommand::Disable { signal, purge } => {
            let kind = SignalKind::from_str(&signal)?;
            let mut config = config_store.load()?;
            config.disable_signal(kind, purge);
            config_store.save(&config)?;

            if purge {
                let outcome = spool.purge_signal(kind)?;
                if kind == SignalKind::Git {
                    GardenStore::new(paths.clone()).clear_git_poll_state()?;
                }
                writeln!(
                    writer,
                    "Disabled signal '{}' and purged {} queued event(s).",
                    kind.as_str(),
                    outcome.removed_events
                )?;
            } else {
                writeln!(writer, "Disabled signal '{}'.", kind.as_str())?;
            }
        }
    }
    Ok(())
}

fn run_daemon(paths: NiwaPaths, clock: &dyn Clock, args: DaemonArgs) -> anyhow::Result<()> {
    match args.action {
        None => {
            let max_wakes = resolve_daemon_max_wakes(args.once, args.max_ticks);
            let _ = run_daemon_loop(paths, clock, &PlatformIdleSource, &ThreadSleeper, max_wakes)?;
            Ok(())
        }
        Some(DaemonCommand::Run { once, max_ticks }) => {
            let max_wakes = resolve_daemon_max_wakes(once, max_ticks);
            let _ = run_daemon_loop(paths, clock, &PlatformIdleSource, &ThreadSleeper, max_wakes)?;
            Ok(())
        }
        Some(DaemonCommand::Install { bin }) => {
            let unit_paths = DaemonUnitPaths::resolve()?;
            let binary_path = match bin {
                Some(override_bin) => override_bin,
                None => std::env::current_exe()?,
            };
            let mut stdout = io::stdout().lock();
            let _ = run_daemon_install(&unit_paths, &binary_path, &mut stdout)?;
            Ok(())
        }
        Some(DaemonCommand::Uninstall) => {
            let unit_paths = DaemonUnitPaths::resolve()?;
            let mut stdout = io::stdout().lock();
            let _ = run_daemon_uninstall(&unit_paths, &mut stdout)?;
            Ok(())
        }
        Some(DaemonCommand::Status) => {
            let unit_paths = DaemonUnitPaths::resolve()?;
            let mut stdout = io::stdout().lock();
            run_daemon_status(&paths, &unit_paths, &mut stdout)
        }
    }
}

const fn resolve_daemon_max_wakes(once: bool, max_ticks: Option<u64>) -> Option<u64> {
    if once {
        Some(1)
    } else {
        max_ticks
    }
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
