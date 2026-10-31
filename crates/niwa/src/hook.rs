//! `niwa hook` CLI subcommands (`zsh`, `bash`, `fish`, `install`, `uninstall`)
//! (PLAN §0.6, §5 & §10 T12).

use clap::{Args, Subcommand};
use std::io::Write;
use std::path::PathBuf;
use std::str::FromStr;
use tg_core::product::CLI_BIN;
use tg_signals::consent::is_signal_enabled;
use tg_signals::SignalKind;
use tg_store::config::ConfigStore;
use tg_store::paths::NiwaPaths;
use tg_store::shell_hook::{
    install_shell_hook, render_hook_snippet, resolve_rc_path, uninstall_shell_hook, ShellKind,
};

#[derive(Args, Debug)]
pub struct HookArgs {
    #[command(subcommand)]
    pub action: HookCommand,
}

#[derive(Subcommand, Debug)]
pub enum HookCommand {
    /// Print the zsh hook snippet that backgrounds `niwa _ingest`.
    Zsh(HookSnippetArgs),
    /// Print the bash hook snippet that backgrounds `niwa _ingest`.
    Bash(HookSnippetArgs),
    /// Print the fish hook snippet that backgrounds `niwa _ingest`.
    Fish(HookSnippetArgs),
    /// Preview or install (`--write-rc`) Niwa's marked hook block in your shell rc file.
    Install(HookInstallArgs),
    /// Remove Niwa's marked hook block from your shell rc file and restore original bytes.
    Uninstall(HookUninstallArgs),
}

#[derive(Args, Debug)]
pub struct HookSnippetArgs {
    /// Override the `niwa` binary path invoked by the snippet (defaults to `command niwa`).
    #[arg(long)]
    pub bin: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct HookInstallArgs {
    /// Target shell (`zsh`, `bash`, or `fish`; defaults to `$SHELL`).
    #[arg(long)]
    pub shell: Option<String>,

    /// Explicit rc file path override (defaults to the shell's user startup file).
    #[arg(long)]
    pub rc: Option<PathBuf>,

    /// Override the `niwa` binary path invoked by the snippet (defaults to `command niwa`).
    #[arg(long)]
    pub bin: Option<PathBuf>,

    /// Write the marked hook block to the rc file (creating `<rc>.niwa.bak` when `<rc>` exists).
    #[arg(long)]
    pub write_rc: bool,
}

#[derive(Args, Debug)]
pub struct HookUninstallArgs {
    /// Target shell (`zsh`, `bash`, or `fish`; defaults to checking `$SHELL` and standard rc files).
    #[arg(long)]
    pub shell: Option<String>,

    /// Explicit rc file path override.
    #[arg(long)]
    pub rc: Option<PathBuf>,
}

/// Executes `niwa hook` subcommands, delegating all rc disk writes to `tg-store`.
pub fn run_hook_with_writer(
    paths: &NiwaPaths,
    args: HookArgs,
    writer: &mut dyn Write,
) -> anyhow::Result<()> {
    match args.action {
        HookCommand::Zsh(s) => {
            write!(
                writer,
                "{}",
                render_hook_snippet(ShellKind::Zsh, s.bin.as_deref())
            )?;
        }
        HookCommand::Bash(s) => {
            write!(
                writer,
                "{}",
                render_hook_snippet(ShellKind::Bash, s.bin.as_deref())
            )?;
        }
        HookCommand::Fish(s) => {
            write!(
                writer,
                "{}",
                render_hook_snippet(ShellKind::Fish, s.bin.as_deref())
            )?;
        }
        HookCommand::Install(install_args) => {
            let shell = match install_args.shell {
                Some(ref raw) => ShellKind::from_str(raw)?,
                None => ShellKind::detect_from_env().ok_or_else(|| {
                    anyhow::anyhow!(
                        "could not detect supported shell from $SHELL; pass --shell zsh, --shell bash, or --shell fish"
                    )
                })?,
            };
            let rc_path = match install_args.rc {
                Some(path) => path,
                None => resolve_rc_path(shell)?,
            };

            let outcome = install_shell_hook(
                shell,
                &rc_path,
                install_args.bin.as_deref(),
                install_args.write_rc,
            )?;

            if !outcome.wrote_rc {
                writeln!(
                    writer,
                    "Preview of marked {shell} hook block for '{}':\n",
                    outcome.rc_path.display()
                )?;
                write!(writer, "{}", outcome.rendered_block)?;
                writeln!(
                    writer,
                    "\nDry run only (rc file was not modified). Re-run `{CLI_BIN} hook install --write-rc` to write this block."
                )?;
            } else {
                writeln!(
                    writer,
                    "Installed {CLI_BIN} {shell} hook block in '{}'.",
                    outcome.rc_path.display()
                )?;
                if let Some(ref bak) = outcome.backup_path {
                    writeln!(writer, "Backup saved to '{}'.", bak.display())?;
                }
                if let Ok(cfg) = ConfigStore::new(paths.clone()).load() {
                    if !is_signal_enabled(&cfg, SignalKind::Shell) {
                        writeln!(
                            writer,
                            "Note: run `{CLI_BIN} signals enable shell` to opt in before shell events are recorded."
                        )?;
                    }
                }
            }
        }
        HookCommand::Uninstall(uninstall_args) => {
            if let Some(rc_path) = uninstall_args.rc {
                let outcome = uninstall_shell_hook(&rc_path)?;
                write_single_uninstall_message(writer, &outcome.rc_path, outcome.removed)?;
            } else if let Some(ref raw_shell) = uninstall_args.shell {
                let shell = ShellKind::from_str(raw_shell)?;
                let rc_path = resolve_rc_path(shell)?;
                let outcome = uninstall_shell_hook(&rc_path)?;
                write_single_uninstall_message(writer, &outcome.rc_path, outcome.removed)?;
            } else {
                let mut candidates = Vec::with_capacity(4);
                if let Some(detected) = ShellKind::detect_from_env() {
                    if let Ok(path) = resolve_rc_path(detected) {
                        candidates.push(path);
                    }
                }
                for shell in ShellKind::ALL {
                    if let Ok(path) = resolve_rc_path(shell) {
                        if !candidates.contains(&path) {
                            candidates.push(path);
                        }
                    }
                }

                let mut removed_any = false;
                for rc_path in candidates {
                    let outcome = uninstall_shell_hook(&rc_path)?;
                    if outcome.removed {
                        removed_any = true;
                        writeln!(
                            writer,
                            "Removed {CLI_BIN} shell hook block from '{}'.",
                            outcome.rc_path.display()
                        )?;
                    }
                }

                if !removed_any {
                    writeln!(writer, "No {CLI_BIN} shell hook block found to remove.")?;
                }
            }
        }
    }
    Ok(())
}

fn write_single_uninstall_message(
    writer: &mut dyn Write,
    rc_path: &std::path::Path,
    removed: bool,
) -> anyhow::Result<()> {
    if removed {
        writeln!(
            writer,
            "Removed {CLI_BIN} shell hook block from '{}'.",
            rc_path.display()
        )?;
    } else {
        writeln!(
            writer,
            "No {CLI_BIN} shell hook block found in '{}'.",
            rc_path.display()
        )?;
    }
    Ok(())
}
