# Privacy & Local Data Specification

Niwa is a local-only idle garden. Privacy is a hard architectural rule enforced in code and CI:

- **Zero network, ever:** Niwa contains no accounts, telemetry, analytics, update checks, or crash reporting. [`scripts/lint-privacy.sh`](../scripts/lint-privacy.sh) and `cargo deny check` fail CI if any networking crate (`reqwest`, `hyper`, `ureq`, `curl`, `socket2`, `rustls`, `native-tls`, etc.) or network API (`std::net`, `URLSession`, `NWConnection`, `CFNetwork`) appears anywhere in the workspace.
- **Opt-in work signals:** Every work signal (`activity`, `git`, `shell`) is **disabled by default**. With all work signals disabled, your garden still grows and blooms from elapsed wall time alone.
- **Explicit consent record:** Nothing is collected or applied for a signal unless you explicitly opt in via `niwa signals enable <name>` (which prints the exact disclosure from [`data/consent.toml`](../data/consent.toml) and records `consented_at` in `config.toml`). Any event with a timestamp earlier than `consented_at` is ignored and discarded.
- **Counts and timestamps only:** The `WorkEvent` wire type in `tg-store` is a `Copy` Rust enum with **zero `String` or `PathBuf` fields** and `#[serde(deny_unknown_fields)]`. JSON lines containing extra fields (such as `"cmd"`, `"cwd"`, `"repo"`, `"message"`, or `"window"`) are rejected as invalid spool lines and compacted away.

---

## What Each Signal Stores

| Signal | Default | Source | Stored in `events.jsonl` | Stored in `config.toml` / `garden.json` | Never Stored |
|---|---|---|---|---|---|
| `time` | Always on | System wall clock when `niwa` runs | *(none)* | `created_at` and `last_tick_at` (`i64` Unix seconds) in `garden.json`; `updated_at` (`i64` Unix seconds) in `snapshot.json` | Any work activity, commands, paths, repos, or window titles |
| `activity` | Disabled (opt-in) | OS user-input idle duration sampled once per 60 s tick | `{"ts":1700000180,"kind":"activity","active":true}` | `signals.activity.enabled` (`bool`), `signals.activity.consented_at` (`i64` Unix seconds) in `config.toml` | Keystrokes, mouse coordinates, app names, window titles, screen contents, URLs, file paths |
| `git` | Disabled (opt-in) | Read-only `git log --since --author=<email> --format=%ct` on configured local repos | `{"ts":1700000120,"kind":"git_commit"}` | `signals.git.enabled` (`bool`), `signals.git.consented_at` (`i64` Unix seconds), and user-configured `git.repos` / `git.author_email` in `config.toml` | Commit messages, commit hashes, diffs, branch names, file names, line counts, remote URLs, or which repo produced an event |
| `shell` | Disabled (opt-in) | Optional shell hook (`zsh`, `bash`, `fish`) running `niwa _ingest --exit <code> --dur <secs>` | `{"ts":1700000060,"kind":"shell","status":"ok","dur_bucket":"fast"}` | `signals.shell.enabled` (`bool`), `signals.shell.consented_at` (`i64` Unix seconds) in `config.toml` | Command text, arguments, flags, working directory (`$PWD`), file paths, environment variables, stdin, stdout, stderr |

### Exact `WorkEvent` Fields (`events.jsonl`)

`events.jsonl` is an append-only JSON Lines spool compacted automatically after catch-up applies completed ticks. Each line is one of three closed schemas:

1. **`activity` (`WorkEvent::Activity`)**
   - `ts` (`i64`): Unix timestamp in seconds when the 60-second tick was sampled.
   - `kind` (`"activity"`): fixed variant tag.
   - `active` (`bool`): `true` if user input occurred during the tick window, `false` otherwise.

2. **`git` (`WorkEvent::GitCommit`, wire `kind`: `git_commit`)**
   - `ts` (`i64`): commit author/committer Unix timestamp in seconds (`%ct`).
   - `kind` (`"git_commit"`): fixed variant tag.

3. **`shell` (`WorkEvent::Shell`)**
   - `ts` (`i64`): command completion Unix timestamp in seconds.
   - `kind` (`"shell"`): fixed variant tag.
   - `status` (`"ok"` | `"err"`): `"ok"` when exit status is `0`, `"err"` for any non-zero exit status.
   - `dur_bucket` (`"instant"` | `"fast"` | `"medium"` | `"long"`):
     - `"instant"`: `< 1 s`
     - `"fast"`: `1 s .. < 10 s`
     - `"medium"`: `10 s .. < 60 s`
     - `"long"`: `>= 60 s`

---

## Aggregate Counters in `garden.json`

To enforce hourly/daily water caps across spool compactions, deduplicate `git` polls without storing commit hashes or paths, and surface data-integrity issues without keeping raw event history, `garden.json` stores small numeric structs only:

- **`water_budget` (`WaterBudgetState`):**
  - `hour_bucket` (`Option<i64>`): UTC hour index (`unix_secs / 3600`).
  - `hour_water_gp` (`u64`): water GP granted in `hour_bucket`.
  - `hour_water_events` (`u64`): count of water-granting events in `hour_bucket` (used for diminishing returns).
  - `day_bucket` (`Option<i64>`): UTC day index (`unix_secs / 86400`).
  - `day_water_gp` (`u64`): water GP granted in `day_bucket`.
  - `last_shell_ts` (`Option<i64>`): Unix timestamp in seconds of the most recent water-granting shell event (used for minimum spacing).
- **`git_poll` (`GitPollState`, omitted when empty):**
  - `consent_anchor` (`Option<i64>`): `signals.git.consented_at` Unix timestamp in seconds anchoring the poll cursor.
  - `last_poll_at` (`Option<i64>`): Unix timestamp in seconds of the most recent read-only `git log` poll.
  - `credited_commits` (`u64`): cumulative count of unique post-consent commits already credited across configured repositories (prevents double-counting on re-polls, `git commit --amend`, and `git rebase`).
- **`diagnostics` (`SaveDiagnostics`):**
  - `dropped_future_events` (`u64`): events skipped because `ts > now`.
  - `dropped_pre_creation_events` (`u64`): events skipped because `ts < created_at`.
  - `corrupt_spool_lines` (`u64`): malformed or schema-rejected lines skipped in `events.jsonl`.
  - `clock_skew_events` (`u64`): runs where the system clock moved backwards relative to `last_tick_at`.

---

## Where Files Live on Disk

Unless overridden by `NIWA_HOME` (used by isolated tests or portable setups), Niwa writes only inside:

- **macOS:** `~/Library/Application Support/Niwa/` (`config.toml`, `garden.json`, `garden.json.bak`, `events.jsonl`, `snapshot.json`, `garden.lock`)
- **Linux / WSL:**
  - Config: `${XDG_CONFIG_HOME:-~/.config}/niwa/config.toml`
  - State & data: `${XDG_STATE_HOME:-~/.local/state}/niwa/` (`garden.json`, `garden.json.bak`, `events.jsonl`, `snapshot.json`, `garden.lock`)

The only files outside those directories Niwa can ever touch—and **only** on explicit opt-in installer subcommands—are:
- Your per-user background service unit when you run `niwa daemon install` / `niwa daemon uninstall` (`~/Library/LaunchAgents/io.github.quiinthieu.niwa.daemon.plist` on macOS, or `${XDG_CONFIG_HOME:-~/.config}/systemd/user/niwa.service` on Linux).
- Your shell startup file when you run `niwa hook install --write-rc` / `niwa hook uninstall` (which writes a clearly marked removable block with a backup).

---

## Inspecting, Disabling, Purging, and Uninstalling

- **Inspect signal status and queued event counts:**
  ```sh
  niwa signals list
  ```
- **Temporarily pause or resume all opt-in signals without losing consent records:**
  ```sh
  niwa signals pause
  niwa signals resume
  ```
- **Stop collecting a signal:**
  ```sh
  niwa signals disable <activity|git|shell>
  ```
- **Stop collecting a signal, clear its consent timestamp, and delete its queued events from `events.jsonl`:**
  ```sh
  niwa signals disable <activity|git|shell> --purge
  ```
- **Remove shell hooks, background service units, and all local state/config (leaving zero residue):**
  ```sh
  niwa hook uninstall
  niwa daemon uninstall
  niwa reset --all
  ```
  `niwa hook uninstall` restores any modified shell startup files (`~/.zshrc`, `~/.bashrc`, `~/.config/fish/config.fish`) byte-for-byte and removes `.niwa.bak` backups. `niwa daemon uninstall` removes the per-user `launchd` plist or `systemd` user unit. `niwa reset --all` deletes `garden.json`, `garden.json.bak`, `snapshot.json`, `events.jsonl`, `config.toml`, `garden.lock`, and any now-empty Niwa data/config directories.
