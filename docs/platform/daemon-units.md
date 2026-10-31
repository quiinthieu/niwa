# Platform Verification: Per-User Daemon Service Units (`launchd` & `systemd`)

- **Verified on:** `2026-10-31`
- **Plan references:** `PLAN.md` §0.7, §0.8, §7 ("Daemon: one coalesced wake per 60 s, no FS polling, and no daemon wakes at all when no signal is enabled"), §8 (`packaging/{launchd,systemd}/`), §10 T10

## Overview

`niwa daemon install` and `niwa daemon uninstall` manage per-user background service definitions without requiring root privileges (`sudo`) and without hardcoding product identifiers (labels and filenames are derived from `tg_core::product::{BUNDLE_ID, CLI_BIN}`).

To satisfy the `PLAN.md` §7 power budget—**zero daemon wakes when no signal is enabled**—neither platform uses an unconditional periodic respawn timer (`StartInterval` or a `.timer` unit):
- On startup and at the top of every 60-second wake cycle, `niwa daemon` checks `any_signal_enabled(&config)`.
- When **no signal is enabled**, `niwa daemon` immediately exits with status **`0`** before acquiring `garden.lock`, polling `IdleSource`, touching `garden.json`, or sleeping.
- Both unit definitions configure restart-on-crash **only** (`KeepAlive.SuccessfulExit = false` on `launchd`; `Restart=on-failure` on `systemd`), so a clean exit `0` keeps the daemon completely dormant with **zero background wakes**.
- When at least one signal **is** enabled, `niwa daemon` runs one coalesced wake every `60 s` (`TICK_SECS`) and performs zero filesystem polling.

---

## macOS: `launchd` Per-User LaunchAgent

- **Verified directory:** `~/Library/LaunchAgents/` (resolved from `$HOME/Library/LaunchAgents`)
- **Label:** `<BUNDLE_ID>.daemon` → `io.github.quiinthieu.niwa.daemon`
- **File path:** `~/Library/LaunchAgents/io.github.quiinthieu.niwa.daemon.plist`
- **Reference template:** [`packaging/launchd/io.github.quiinthieu.niwa.daemon.plist.in`](../../packaging/launchd/io.github.quiinthieu.niwa.daemon.plist.in)
- **Verified `launchd.plist(5)` keys (`2026-10-31`):**
  - `Label` (`string`): `io.github.quiinthieu.niwa.daemon` — unique reverse-DNS job label matching filename convention.
  - `ProgramArguments` (`array` of `string`): `["<path/to/niwa>", "daemon"]` — direct `execve` argument vector without shell interpolation.
  - `ProcessType` (`string`): `Background` — informs XNU / `launchd` QoS scheduling to coalesce timers and run at energy-efficient background priority (`PLAN.md` §7).
  - `RunAtLoad` (`bool`): `true` — launches once when loaded or at user GUI login; exits `0` immediately and stays stopped if no work signal is enabled.
  - `KeepAlive` (`dict`): `{ "SuccessfulExit": false }` — restarts `niwa daemon` only if it terminates abnormally (non-zero exit status or crash signal), never after a clean `0` exit.
- **Manual user activation / deactivation commands (`launchctl(1)`):**
  ```sh
  launchctl bootstrap "gui/$(id -u)" ~/Library/LaunchAgents/io.github.quiinthieu.niwa.daemon.plist
  launchctl bootout "gui/$(id -u)" ~/Library/LaunchAgents/io.github.quiinthieu.niwa.daemon.plist
  ```
- **Official documentation URLs & man pages (verified `2026-10-31`):**
  - macOS manual pages: `launchd.plist(5)` and `launchctl(1)`
  - Apple Daemons and Services Programming Guide ("Creating Launch Daemons and Agents", per-user agents in `~/Library/LaunchAgents`): `https://developer.apple.com/library/archive/documentation/MacOSX/Conceptual/BPSystemStartup/Chapters/CreatingLaunchdJobs.html`

---

## Linux: `systemd` Per-User Service Unit

- **Verified directory:** `${XDG_CONFIG_HOME:-~/.config}/systemd/user/`
- **Unit name:** `<CLI_BIN>.service` → `niwa.service`
- **File path:** `${XDG_CONFIG_HOME:-~/.config}/systemd/user/niwa.service`
- **Reference template:** [`packaging/systemd/niwa.service.in`](../../packaging/systemd/niwa.service.in)
- **Verified `systemd.unit(5)` & `systemd.service(5)` directives (`2026-10-31`):**
  - `[Unit]`
    - `Description=Niwa background garden daemon`
    - `Documentation=https://github.com/quiinthieu/niwa`
  - `[Service]`
    - `Type=simple`
    - `ExecStart=<escaped-path-to-niwa> daemon` (paths containing whitespace or quotes are double-quoted and `\` / `"` / `%` escaped per `systemd.service(5)` / `systemd.unit(5)` specifier rules)
    - `Restart=on-failure` — restarts only on non-zero exit code, unclean signal, or timeout; does **not** restart when `niwa daemon` exits `0` because no signals are enabled.
    - `RestartSec=10`
  - `[Install]`
    - `WantedBy=default.target`
- **Manual user activation / deactivation commands (`systemctl(1)`):**
  ```sh
  systemctl --user daemon-reload
  systemctl --user enable --now niwa.service
  systemctl --user disable --now niwa.service
  ```
- **Official documentation URLs (verified `2026-10-31`):**
  - `systemd.unit(5)` ("Unit File Load Path" → User Configuration `${XDG_CONFIG_HOME}/systemd/user` and `~/.config/systemd/user`): `https://www.freedesktop.org/software/systemd/man/latest/systemd.unit.html`
  - `systemd.service(5)` (`Type=`, `ExecStart=`, `Restart=on-failure`): `https://www.freedesktop.org/software/systemd/man/latest/systemd.service.html`
