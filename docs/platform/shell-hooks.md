# Platform Verification: Shell Hooks (`zsh`, `bash`, `fish`)

- **Verified on:** `2026-10-31`
- **Plan references:** `PLAN.md` §0.4, §0.6, §0.8, §5 (`zsh precmd, bash PROMPT_COMMAND, fish fish_postexec *(verify)*`), §7 (`_ingest` p95 latency target `< 10 ms`), §9, §10 T12

## Overview

`niwa hook zsh|bash|fish` prints a shell snippet that records completed interactive command exit codes and coarse wall-clock durations by backgrounding `niwa _ingest --exit <code> --dur <secs>`.

Hard invariants enforced across all three shells (`PLAN.md` §0.4, §0.6, §5, §7):
1. **Zero command text or path access:** No hook reads positional command arguments (`$1`..`$3` in `zsh` `preexec`, `$argv` in `fish_postexec`), `$PWD`, or shell history. `niwa _ingest` accepts only `--exit <i32>` and `--dur <secs>` and stores only `(ts, status: ok|err, dur_bucket: instant|fast|medium|long)` in `events.jsonl`.
2. **Non-blocking & silent:** `niwa _ingest` is spawned asynchronously in the background with stdout and stderr redirected to `/dev/null` and job-control notifications suppressed, so prompt rendering never waits on disk I/O and missing/disabled configurations emit zero terminal output.
3. **Exit status preservation:** Every prompt hook captures `$?` / `$status` as its first operation and returns that exact integer so user prompts (`%?`, `$?`, `$status`) and downstream hooks observe the original command result.
4. **Explicit `--write-rc` opt-in & byte-for-byte uninstall:** `niwa hook install` previews changes by default and edits the user's rc file **only** when `--write-rc` is passed, writing a sibling `<rc>.niwa.bak` backup (when `<rc>` existed) and wrapping the snippet inside `# >>> niwa shell hook >>>` … `# <<< niwa shell hook <<<` with restoration metadata so `niwa hook uninstall` restores the pre-install rc file byte for byte.

---

## 1. Zsh (`preexec` / `precmd` via `add-zsh-hook`)

- **User startup file:** `${ZDOTDIR:-$HOME}/.zshrc`
- **Verified mechanics (`2026-10-31`):**
  - `autoload -Uz add-zsh-hook` loads Zsh's standard hook registration helper so Niwa composes cleanly with `precmd_functions` and `preexec_functions` arrays instead of clobbering user `precmd()` / `preexec()` definitions.
  - `preexec` fires right after an interactive command line is read and is about to be executed (never on blank Enter presses or during `.zshrc` startup). `__niwa_preexec` records `$SECONDS` without inspecting `$1`, `$2`, or `$3`.
  - `precmd` fires before each prompt display. Placing `__niwa_precmd` at the head of `precmd_functions` guarantees `local __niwa_ec=$?` captures the command's exit code before other prompt themes overwrite `$?`, and `return $__niwa_ec` forwards that exit status to subsequent hooks.
  - Zsh enables `BG_NICE` (`nice(5)`) and job notifications (`NOTIFY`, `MONITOR`) by default in interactive shells, which can print `nice(5) failed: operation not permitted` in restricted sandboxes/containers or `[1] + done` messages. Setting `emulate -L zsh` and `setopt NO_BG_NICE NO_NOTIFY NO_MONITOR` inside `__niwa_precmd` and launching `_ingest` inside a subshell `( command niwa _ingest ... >/dev/null 2>&1 & ) 2>/dev/null` suppresses all job table entries and `nice(5)` errors.
  - Non-TTY automated test note: interactive `zsh` (`-i`) activates ZLE on `/dev/tty` unless `+Z` (`--no-zle`) is passed when feeding commands over piped stdin.
- **Official documentation URLs (verified `2026-10-31`):**
  - Zsh Special Functions (`precmd`, `preexec`, `precmd_functions`, `preexec_functions`): `https://zsh.sourceforge.io/Doc/Release/Functions.html`
  - Zsh User Contributions (`add-zsh-hook`): `https://zsh.sourceforge.io/Doc/Release/User-Contributions.html`
  - Zsh Parameters Used By The Shell (`SECONDS`, `ZDOTDIR`): `https://zsh.sourceforge.io/Doc/Release/Parameters.html`
  - Zsh Options (`BG_NICE`, `NOTIFY`, `MONITOR`, `ZLE`): `https://zsh.sourceforge.io/Doc/Release/Options.html`

---

## 2. Bash (`DEBUG` trap + `PROMPT_COMMAND`)

- **User startup file:** `$HOME/.bashrc`
- **Compatibility range:** GNU Bash `3.2` (macOS system `/bin/bash`) through GNU Bash `5.x` (Linux `/bin/bash` and Homebrew Bash).
  - *Why not `PS0` or `$EPOCHREALTIME`?* macOS ships Bash `3.2.57` as `/bin/bash`, whereas `PS0` requires Bash `4.4+` and `$EPOCHREALTIME` requires Bash `5.0+` (and formats fractional seconds using `,` under comma-decimal `LC_NUMERIC` locales). `$SECONDS` is available in every Bash release since `3.2`, is locale-independent, and aligns directly with `DurationBucket`'s integer-second thresholds (`1 s`, `10 s`, `60 s`).
- **Verified mechanics (`2026-10-31`):**
  - `PROMPT_COMMAND` is a scalar command string in Bash `3.2–5.0` and may be either a scalar string or an array (`declare -a PROMPT_COMMAND`) in Bash `5.1+`. The installer inspects `declare -p PROMPT_COMMAND` and wraps existing entries with `__niwa_precmd` first (capturing `local __niwa_ec=$?` immediately) and `__niwa_postprompt` last (clearing `__niwa_in_prompt=""` while preserving `$?` via `return $__niwa_ec`).
  - `__niwa_in_prompt` starts at `1` while `.bashrc` is sourced so trailing commands in `.bashrc` after the Niwa block do not arm `__niwa_cmd_start` prior to the first prompt.
  - `trap '__niwa_preexec' DEBUG` captures `__niwa_cmd_start=$SECONDS` on the first simple command of an interactive pipeline while ignoring:
    - programmable tab-completion invocations (`[[ -n "${COMP_LINE:-}" ]]`),
    - commands executed while rendering the prompt (`[[ -n "${__niwa_in_prompt:-}" ]]`), and
    - blank-line prompt transitions where Bash triggers `DEBUG` as `PROMPT_COMMAND` begins (`case "${BASH_COMMAND:-}" in __niwa_precmd*|__niwa_postprompt*) return 0 ;; esac`).
  - Backgrounding `_ingest` inside a subshell `( command niwa _ingest ... >/dev/null 2>&1 & ) 2>/dev/null` detaches it from the interactive shell's job table so `[1]+ Done` is never printed.
- **Official documentation URLs (verified `2026-10-31`):**
  - GNU Bash Manual — Bash Variables (`PROMPT_COMMAND`, `SECONDS`, `BASH_COMMAND`, `COMP_LINE`): `https://www.gnu.org/software/bash/manual/html_node/Bash-Variables.html`
  - GNU Bash Manual — Bourne Shell Builtins (`trap` `DEBUG`): `https://www.gnu.org/software/bash/manual/html_node/Bourne-Shell-Builtins.html`
  - GNU Bash Manual — Bash Startup Files (`~/.bashrc`): `https://www.gnu.org/software/bash/manual/html_node/Bash-Startup-Files.html`

---

## 3. Fish (`--on-event fish_postexec`)

- **User startup file:** `${XDG_CONFIG_HOME:-$HOME/.config}/fish/config.fish`
- **Verified mechanics (`2026-10-31`):**
  - `function __niwa_postexec --on-event fish_postexec` fires automatically after each interactive command line finishes (never on blank Enter presses).
  - Fish passes the command text in `$argv`, which `__niwa_postexec` **never reads**.
  - Inside `__niwa_postexec`, `$status` holds the command's exit code and `$CMD_DURATION` holds its wall-clock runtime in integer milliseconds. Both are copied into local variables (`set -l __niwa_ec $status` and `set -l __niwa_dur_ms $CMD_DURATION`) before running `test` or `math` so `$status` cannot be clobbered.
  - `math "$__niwa_dur_ms / 1000"` converts milliseconds to floating-point seconds using `.` regardless of locale.
  - `command niwa _ingest --exit "$__niwa_ec" --dur "$__niwa_dur" >/dev/null 2>&1 &` followed by `disown $__niwa_pid 2>/dev/null` removes the background process from Fish's job list so exit notifications are never printed, and `return $__niwa_ec` preserves `$status` for `fish_prompt`.
- **Official documentation URLs (verified `2026-10-31`):**
  - Fish Documentation — The Fish Language (`fish_postexec` event handlers, `$status`, `$CMD_DURATION`, configuration files): `https://fishshell.com/docs/current/language.html`
  - Fish `function` builtin (`--on-event`): `https://fishshell.com/docs/current/cmds/function.html`
  - Fish `disown` builtin: `https://fishshell.com/docs/current/cmds/disown.html`
