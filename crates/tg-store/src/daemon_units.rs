//! Per-user `launchd` (macOS) and `systemd` (Linux) daemon unit path resolution,
//! rendering, atomic installation, and removal (PLAN §0.6, §7, §8 & §10 T10).
//!
//! Verified platform paths and directives are recorded in `docs/platform/daemon-units.md`.

use crate::atomic::atomic_write_bytes;
use crate::error::StoreError;
use crate::paths::PlatformKind;
use directories::BaseDirs;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use tg_core::product::{BUNDLE_ID, CLI_BIN, DISPLAY_NAME};

/// Returns the reverse-DNS `launchd` job label (`<BUNDLE_ID>.daemon`).
#[must_use]
pub fn launchd_daemon_label() -> String {
    format!("{BUNDLE_ID}.daemon")
}

/// Returns the `launchd` plist filename (`<BUNDLE_ID>.daemon.plist`).
#[must_use]
pub fn launchd_plist_filename() -> String {
    format!("{}.plist", launchd_daemon_label())
}

/// Returns the `systemd --user` service filename (`<CLI_BIN>.service`).
#[must_use]
pub fn systemd_service_filename() -> String {
    format!("{CLI_BIN}.service")
}

/// Resolved per-user daemon service unit path and identifier for macOS or Linux.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DaemonUnitPaths {
    platform: PlatformKind,
    unit_identifier: String,
    unit_path: PathBuf,
}

impl DaemonUnitPaths {
    /// Resolves the per-user daemon unit path from the current host environment.
    ///
    /// Honours `HOME` and `XDG_CONFIG_HOME` ahead of [`BaseDirs`] so isolated [`TestEnv`] runs
    /// never touch the real user `$HOME`.
    pub fn resolve() -> Result<Self, StoreError> {
        let base_dirs = BaseDirs::new();
        let fallback_home = base_dirs.as_ref().map(BaseDirs::home_dir);
        Self::resolve_from_env(
            PlatformKind::current(),
            |key| std::env::var_os(key),
            fallback_home,
        )
    }

    /// Pure, deterministic path resolver accepting an injected platform, env lookup, and fallback home.
    pub fn resolve_from_env<F>(
        platform: PlatformKind,
        mut get_env: F,
        fallback_home: Option<&Path>,
    ) -> Result<Self, StoreError>
    where
        F: FnMut(&str) -> Option<OsString>,
    {
        let home = non_empty_os(get_env("HOME"))
            .map(PathBuf::from)
            .or_else(|| fallback_home.map(Path::to_path_buf))
            .ok_or(StoreError::MissingHomeDirectory)?;

        match platform {
            PlatformKind::MacOs => {
                let unit_identifier = launchd_daemon_label();
                let unit_path = home
                    .join("Library")
                    .join("LaunchAgents")
                    .join(format!("{unit_identifier}.plist"));
                Ok(Self {
                    platform,
                    unit_identifier,
                    unit_path,
                })
            }
            PlatformKind::Linux => {
                let unit_identifier = systemd_service_filename();
                let config_root = non_empty_os(get_env("XDG_CONFIG_HOME"))
                    .map(PathBuf::from)
                    .unwrap_or_else(|| home.join(".config"));
                let unit_path = config_root
                    .join("systemd")
                    .join("user")
                    .join(&unit_identifier);
                Ok(Self {
                    platform,
                    unit_identifier,
                    unit_path,
                })
            }
        }
    }

    #[must_use]
    pub const fn platform(&self) -> PlatformKind {
        self.platform
    }

    #[must_use]
    pub fn unit_identifier(&self) -> &str {
        &self.unit_identifier
    }

    #[must_use]
    pub fn unit_path(&self) -> &Path {
        &self.unit_path
    }

    #[must_use]
    pub fn is_installed(&self) -> bool {
        self.unit_path.is_file()
    }
}

/// Summary returned after atomically installing the per-user daemon unit file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallDaemonUnitOutcome {
    pub platform: PlatformKind,
    pub unit_identifier: String,
    pub unit_path: PathBuf,
    pub contents: String,
}

/// Summary returned after removing the per-user daemon unit file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UninstallDaemonUnitOutcome {
    pub platform: PlatformKind,
    pub unit_identifier: String,
    pub unit_path: PathBuf,
    pub removed: bool,
}

/// Renders the per-user macOS `launchd` XML plist for `binary_path`.
///
/// Uses `ProcessType = Background` and `KeepAlive.SuccessfulExit = false` (without `StartInterval`)
/// so a clean `0` exit when no signals are enabled results in zero background wakes (`PLAN.md` §7).
#[must_use]
pub fn render_launchd_plist(binary_path: &Path) -> String {
    let label = xml_escape(&launchd_daemon_label());
    let bin = xml_escape(&binary_path.to_string_lossy());
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{bin}</string>
        <string>daemon</string>
    </array>
    <key>ProcessType</key>
    <string>Background</string>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
</dict>
</plist>
"#
    )
}

/// Renders the per-user Linux `systemd --user` service unit for `binary_path`.
///
/// Uses `Restart=on-failure` (never `Restart=always` or a periodic `.timer`) so a clean `0` exit
/// when no signals are enabled leaves the unit inactive with zero background wakes (`PLAN.md` §7).
#[must_use]
pub fn render_systemd_service(binary_path: &Path) -> String {
    let exec_bin = escape_systemd_exec_arg(&binary_path.to_string_lossy());
    format!(
        r#"[Unit]
Description={DISPLAY_NAME} background garden daemon
Documentation=https://github.com/quiinthieu/niwa

[Service]
Type=simple
ExecStart={exec_bin} daemon
Restart=on-failure
RestartSec=10

[Install]
WantedBy=default.target
"#
    )
}

/// Renders the appropriate user daemon unit definition for `platform`.
#[must_use]
pub fn render_daemon_unit(platform: PlatformKind, binary_path: &Path) -> String {
    match platform {
        PlatformKind::MacOs => render_launchd_plist(binary_path),
        PlatformKind::Linux => render_systemd_service(binary_path),
    }
}

/// Atomically writes the rendered `launchd` plist or `systemd` user service to `unit_paths`.
pub fn install_daemon_unit(
    unit_paths: &DaemonUnitPaths,
    binary_path: &Path,
) -> Result<InstallDaemonUnitOutcome, StoreError> {
    let contents = render_daemon_unit(unit_paths.platform, binary_path);
    atomic_write_bytes(&unit_paths.unit_path, None, contents.as_bytes(), None)?;
    Ok(InstallDaemonUnitOutcome {
        platform: unit_paths.platform,
        unit_identifier: unit_paths.unit_identifier.clone(),
        unit_path: unit_paths.unit_path.clone(),
        contents,
    })
}

/// Removes the per-user `launchd` plist or `systemd` user service if present (idempotent),
/// pruning empty unit directories created during installation.
pub fn uninstall_daemon_unit(
    unit_paths: &DaemonUnitPaths,
) -> Result<UninstallDaemonUnitOutcome, StoreError> {
    let removed = match fs::remove_file(&unit_paths.unit_path) {
        Ok(()) => true,
        Err(err) if err.kind() == io::ErrorKind::NotFound => false,
        Err(source) => {
            return Err(StoreError::Io {
                path: unit_paths.unit_path.clone(),
                action: "remove user daemon unit file",
                source,
            });
        }
    };
    if removed {
        match unit_paths.platform {
            PlatformKind::MacOs => {
                if let Some(launch_agents) = unit_paths.unit_path.parent() {
                    let _ = fs::remove_dir(launch_agents);
                }
            }
            PlatformKind::Linux => {
                if let Some(systemd_user) = unit_paths.unit_path.parent() {
                    let _ = fs::remove_dir(systemd_user);
                    if let Some(systemd_dir) = systemd_user.parent() {
                        let _ = fs::remove_dir(systemd_dir);
                    }
                }
            }
        }
    }
    Ok(UninstallDaemonUnitOutcome {
        platform: unit_paths.platform,
        unit_identifier: unit_paths.unit_identifier.clone(),
        unit_path: unit_paths.unit_path.clone(),
        removed,
    })
}

fn non_empty_os(val: Option<OsString>) -> Option<OsString> {
    val.filter(|v| !v.is_empty())
}

fn xml_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    out
}

fn escape_systemd_exec_arg(input: &str) -> String {
    let needs_quotes = input
        .chars()
        .any(|ch| ch.is_ascii_whitespace() || matches!(ch, '\\' | '"' | '\'' | ';'));

    let mut escaped = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '%' => escaped.push_str("%%"),
            _ => escaped.push(ch),
        }
    }

    if needs_quotes {
        format!("\"{escaped}\"")
    } else {
        escaped
    }
}
