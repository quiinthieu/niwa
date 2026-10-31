//! Shell hook snippet rendering (`zsh`, `bash`, `fish`), rc path resolution, and
//! byte-exact marked-block installation and removal with `.niwa.bak` backups
//! (PLAN §0.4, §0.6, §5, §9 & §10 T12).
//!
//! Verified shell mechanics and official documentation URLs live in
//! `docs/platform/shell-hooks.md`.

use crate::atomic::atomic_write_bytes;
use crate::error::StoreError;
use directories::BaseDirs;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use tg_core::product::CLI_BIN;

/// Opening delimiter line of Niwa's removable shell rc block.
pub const HOOK_BLOCK_START: &str = "# >>> niwa shell hook >>>";

/// Closing delimiter line of Niwa's removable shell rc block.
pub const HOOK_BLOCK_END: &str = "# <<< niwa shell hook <<<";

const META_LINE_PREFIX: &[u8] = b"# niwa-rc-meta:";

/// Supported interactive shells for `niwa hook` (`zsh`, `bash`, `fish`).
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ShellKind {
    Zsh,
    Bash,
    Fish,
}

impl ShellKind {
    pub const ALL: [Self; 3] = [Self::Zsh, Self::Bash, Self::Fish];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Zsh => "zsh",
            Self::Bash => "bash",
            Self::Fish => "fish",
        }
    }

    /// Detects the user's interactive shell from `$SHELL` in the current environment.
    #[must_use]
    pub fn detect_from_env() -> Option<Self> {
        let val = std::env::var_os("SHELL")?;
        Self::detect_from_shell_env(&val)
    }

    /// Parses a `$SHELL` value (e.g. `/bin/zsh`, `/usr/local/bin/fish`, `-bash`).
    #[must_use]
    pub fn detect_from_shell_env(shell_var: &OsStr) -> Option<Self> {
        if shell_var.is_empty() {
            return None;
        }
        let base = Path::new(shell_var).file_name().unwrap_or(shell_var);
        let name = base.to_string_lossy();
        let normalized = name.trim().trim_start_matches('-');
        Self::from_str(normalized).ok()
    }
}

impl fmt::Display for ShellKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error returned when parsing a shell name on the CLI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseShellKindError(pub String);

impl fmt::Display for ParseShellKindError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unknown shell '{}' (expected one of: zsh, bash, fish)",
            self.0
        )
    }
}

impl std::error::Error for ParseShellKindError {}

impl FromStr for ShellKind {
    type Err = ParseShellKindError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "zsh" => Ok(Self::Zsh),
            "bash" => Ok(Self::Bash),
            "fish" => Ok(Self::Fish),
            _ => Err(ParseShellKindError(s.to_string())),
        }
    }
}

/// Resolves the per-user startup rc file for `shell` from the host environment.
///
/// Honours `HOME`, `ZDOTDIR`, and `XDG_CONFIG_HOME` ahead of [`BaseDirs`] so isolated
/// `TestEnv` runs never touch the real user `$HOME`.
pub fn resolve_rc_path(shell: ShellKind) -> Result<PathBuf, StoreError> {
    if non_empty_os(std::env::var_os("HOME")).is_some() {
        return resolve_rc_path_from_env(shell, |key| std::env::var_os(key), None);
    }
    let base_dirs = BaseDirs::new();
    let fallback_home = base_dirs.as_ref().map(BaseDirs::home_dir);
    resolve_rc_path_from_env(shell, |key| std::env::var_os(key), fallback_home)
}

/// Pure, deterministic rc file path resolver accepting an injected env lookup and fallback home.
pub fn resolve_rc_path_from_env<F>(
    shell: ShellKind,
    mut get_env: F,
    fallback_home: Option<&Path>,
) -> Result<PathBuf, StoreError>
where
    F: FnMut(&str) -> Option<OsString>,
{
    match shell {
        ShellKind::Zsh => {
            if let Some(zdotdir) = non_empty_os(get_env("ZDOTDIR")) {
                return Ok(PathBuf::from(zdotdir).join(".zshrc"));
            }
            let home = resolve_home(&mut get_env, fallback_home)?;
            Ok(home.join(".zshrc"))
        }
        ShellKind::Bash => {
            let home = resolve_home(&mut get_env, fallback_home)?;
            Ok(home.join(".bashrc"))
        }
        ShellKind::Fish => {
            if let Some(xdg_config) = non_empty_os(get_env("XDG_CONFIG_HOME")) {
                return Ok(PathBuf::from(xdg_config).join("fish").join("config.fish"));
            }
            let home = resolve_home(&mut get_env, fallback_home)?;
            Ok(home.join(".config").join("fish").join("config.fish"))
        }
    }
}

/// Returns the sibling `.niwa.bak` backup path for `rc_path` (e.g. `~/.zshrc.niwa.bak`).
#[must_use]
pub fn backup_path_for_rc(rc_path: &Path) -> PathBuf {
    let mut os = rc_path.as_os_str().to_os_string();
    os.push(".niwa.bak");
    PathBuf::from(os)
}

/// Renders the standalone shell hook snippet for `shell` (`niwa hook zsh|bash|fish`).
///
/// Never reads command text, positional arguments, working directory, or history; only forwards
/// exit code (`--exit`) and elapsed wall seconds (`--dur`) to `niwa _ingest` in the background.
#[must_use]
pub fn render_hook_snippet(shell: ShellKind, binary_override: Option<&Path>) -> String {
    let exec_cmd = quote_executable(shell, binary_override);
    match shell {
        ShellKind::Zsh => render_zsh_snippet(&exec_cmd),
        ShellKind::Bash => render_bash_snippet(&exec_cmd),
        ShellKind::Fish => render_fish_snippet(&exec_cmd),
    }
}

/// Summary returned by [`install_shell_hook`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallShellHookOutcome {
    pub shell: ShellKind,
    pub rc_path: PathBuf,
    pub backup_path: Option<PathBuf>,
    pub rendered_block: String,
    pub wrote_rc: bool,
}

/// Summary returned by [`uninstall_shell_hook`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UninstallShellHookOutcome {
    pub rc_path: PathBuf,
    pub removed: bool,
}

/// Previews (`write_rc = false`) or atomically installs (`write_rc = true`) Niwa's marked shell
/// hook block into `rc_path`, writing a `<rc_path>.niwa.bak` backup when `rc_path` pre-existed.
pub fn install_shell_hook(
    shell: ShellKind,
    rc_path: &Path,
    binary_override: Option<&Path>,
    write_rc: bool,
) -> Result<InstallShellHookOutcome, StoreError> {
    let existing_bytes = read_optional_bytes(rc_path)?;
    let existing_span = match existing_bytes.as_deref() {
        Some(bytes) => scan_marked_block(rc_path, bytes)?,
        None => None,
    };

    let meta = match (&existing_bytes, &existing_span) {
        (_, Some(span)) => span.meta,
        (None, None) => BlockMeta {
            prefix: PrefixKind::None,
            created: true,
        },
        (Some(bytes), None) => BlockMeta {
            prefix: PrefixKind::for_existing_bytes(bytes),
            created: false,
        },
    };

    let snippet = render_hook_snippet(shell, binary_override);
    let rendered_block = render_marked_block(&snippet, meta);

    if !write_rc {
        return Ok(InstallShellHookOutcome {
            shell,
            rc_path: rc_path.to_path_buf(),
            backup_path: None,
            rendered_block,
            wrote_rc: false,
        });
    }

    let existing_perms = fs::metadata(rc_path).ok().map(|m| m.permissions());

    let backup_path = if let Some(ref orig_bytes) = existing_bytes {
        let bak = backup_path_for_rc(rc_path);
        if let Some(ref span) = existing_span {
            if !bak.exists() {
                let pre_niwa = strip_span(orig_bytes, span);
                atomic_write_bytes(&bak, None, &pre_niwa, None)?;
                apply_permissions(&bak, existing_perms.as_ref());
            }
        } else {
            atomic_write_bytes(&bak, None, orig_bytes, None)?;
            apply_permissions(&bak, existing_perms.as_ref());
        }
        Some(bak)
    } else {
        None
    };

    let new_bytes = match (&existing_bytes, &existing_span) {
        (Some(orig_bytes), Some(span)) => {
            let mut buf = Vec::with_capacity(
                orig_bytes.len() - (span.end - span.start) + rendered_block.len(),
            );
            buf.extend_from_slice(&orig_bytes[..span.start]);
            buf.extend_from_slice(rendered_block.as_bytes());
            buf.extend_from_slice(&orig_bytes[span.end..]);
            buf
        }
        (Some(orig_bytes), None) => {
            let prefix_bytes = meta.prefix.as_bytes();
            let mut buf =
                Vec::with_capacity(orig_bytes.len() + prefix_bytes.len() + rendered_block.len());
            buf.extend_from_slice(orig_bytes);
            buf.extend_from_slice(prefix_bytes);
            buf.extend_from_slice(rendered_block.as_bytes());
            buf
        }
        (None, _) => rendered_block.as_bytes().to_vec(),
    };

    atomic_write_bytes(rc_path, None, &new_bytes, None)?;
    apply_permissions(rc_path, existing_perms.as_ref());

    Ok(InstallShellHookOutcome {
        shell,
        rc_path: rc_path.to_path_buf(),
        backup_path,
        rendered_block,
        wrote_rc: true,
    })
}

/// Removes Niwa's marked shell hook block from `rc_path` (if present), restoring pre-install
/// contents byte-for-byte and cleaning up `<rc_path>.niwa.bak`.
pub fn uninstall_shell_hook(rc_path: &Path) -> Result<UninstallShellHookOutcome, StoreError> {
    let Some(bytes) = read_optional_bytes(rc_path)? else {
        remove_file_if_present(&backup_path_for_rc(rc_path), "remove shell hook backup")?;
        return Ok(UninstallShellHookOutcome {
            rc_path: rc_path.to_path_buf(),
            removed: false,
        });
    };

    let Some(span) = scan_marked_block(rc_path, &bytes)? else {
        return Ok(UninstallShellHookOutcome {
            rc_path: rc_path.to_path_buf(),
            removed: false,
        });
    };

    let existing_perms = fs::metadata(rc_path).ok().map(|m| m.permissions());
    let restored = strip_span(&bytes, &span);

    if span.meta.created && restored.is_empty() {
        remove_file_if_present(rc_path, "remove created shell rc file")?;
    } else {
        atomic_write_bytes(rc_path, None, &restored, None)?;
        apply_permissions(rc_path, existing_perms.as_ref());
    }

    remove_file_if_present(&backup_path_for_rc(rc_path), "remove shell hook backup")?;

    Ok(UninstallShellHookOutcome {
        rc_path: rc_path.to_path_buf(),
        removed: true,
    })
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum PrefixKind {
    None,
    Lf,
    TwoLf,
}

impl PrefixKind {
    fn for_existing_bytes(bytes: &[u8]) -> Self {
        if bytes.is_empty() || bytes.ends_with(b"\n\n") {
            Self::None
        } else if bytes.ends_with(b"\n") {
            Self::Lf
        } else {
            Self::TwoLf
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Lf => "lf",
            Self::TwoLf => "2lf",
        }
    }

    const fn as_bytes(self) -> &'static [u8] {
        match self {
            Self::None => b"",
            Self::Lf => b"\n",
            Self::TwoLf => b"\n\n",
        }
    }

    fn from_token(token: &str) -> Option<Self> {
        match token {
            "none" => Some(Self::None),
            "lf" => Some(Self::Lf),
            "2lf" => Some(Self::TwoLf),
            _ => None,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct BlockMeta {
    prefix: PrefixKind,
    created: bool,
}

impl Default for BlockMeta {
    fn default() -> Self {
        Self {
            prefix: PrefixKind::Lf,
            created: false,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct BlockSpan {
    start: usize,
    end: usize,
    meta: BlockMeta,
}

fn render_marked_block(snippet: &str, meta: BlockMeta) -> String {
    let created_flag = u8::from(meta.created);
    let trimmed = snippet.trim_end_matches('\n');
    format!(
        "{HOOK_BLOCK_START}\n# niwa-rc-meta: prefix={} created={created_flag}\n{trimmed}\n{HOOK_BLOCK_END}\n",
        meta.prefix.as_str()
    )
}

fn scan_marked_block(rc_path: &Path, bytes: &[u8]) -> Result<Option<BlockSpan>, StoreError> {
    let mut starts = Vec::new();
    let mut ends = Vec::new();
    let mut line_start = 0_usize;

    while line_start < bytes.len() {
        let rel_nl = bytes[line_start..].iter().position(|&b| b == b'\n');
        let (line_end_no_nl, next_line_start) = match rel_nl {
            Some(pos) => {
                let nl_idx = line_start + pos;
                let trimmed_cr = if nl_idx > line_start && bytes[nl_idx - 1] == b'\r' {
                    nl_idx - 1
                } else {
                    nl_idx
                };
                (trimmed_cr, nl_idx + 1)
            }
            None => (bytes.len(), bytes.len()),
        };

        let line = trim_trailing_ascii_spaces(&bytes[line_start..line_end_no_nl]);
        if line == HOOK_BLOCK_START.as_bytes() {
            starts.push((line_start, next_line_start));
        } else if line == HOOK_BLOCK_END.as_bytes() {
            ends.push((line_start, next_line_start));
        }

        line_start = next_line_start;
    }

    if starts.is_empty() && ends.is_empty() {
        return Ok(None);
    }

    if starts.len() != 1 || ends.len() != 1 {
        return Err(StoreError::MalformedShellHookBlock {
            path: rc_path.to_path_buf(),
            reason: format!(
                "found {} start marker(s) and {} end marker(s); expected 1 matching pair",
                starts.len(),
                ends.len()
            ),
        });
    }

    let (start_idx, after_start_line) = starts[0];
    let (end_line_start, end_idx) = ends[0];
    if end_line_start <= start_idx {
        return Err(StoreError::MalformedShellHookBlock {
            path: rc_path.to_path_buf(),
            reason: "end marker appears before start marker".to_string(),
        });
    }

    let body = &bytes[after_start_line..end_line_start];
    let meta = parse_block_meta(body).unwrap_or_default();

    Ok(Some(BlockSpan {
        start: start_idx,
        end: end_idx,
        meta,
    }))
}

fn parse_block_meta(body: &[u8]) -> Option<BlockMeta> {
    let first_line_end = body.iter().position(|&b| b == b'\n').unwrap_or(body.len());
    let mut line = &body[..first_line_end];
    if line.ends_with(b"\r") {
        line = &line[..line.len() - 1];
    }
    let rest = line.strip_prefix(META_LINE_PREFIX)?;
    let text = std::str::from_utf8(rest).ok()?;

    let mut prefix = None;
    let mut created = None;
    for part in text.split_ascii_whitespace() {
        if let Some(val) = part.strip_prefix("prefix=") {
            prefix = PrefixKind::from_token(val);
        } else if let Some(val) = part.strip_prefix("created=") {
            created = match val {
                "1" | "true" => Some(true),
                "0" | "false" => Some(false),
                _ => None,
            };
        }
    }

    Some(BlockMeta {
        prefix: prefix?,
        created: created?,
    })
}

fn strip_span(bytes: &[u8], span: &BlockSpan) -> Vec<u8> {
    let raw_before = &bytes[..span.start];
    let after = &bytes[span.end..];

    let prefix_bytes = span.meta.prefix.as_bytes();
    let stripped_before = raw_before.strip_suffix(prefix_bytes).unwrap_or(raw_before);

    let needs_line_sep =
        !stripped_before.is_empty() && !stripped_before.ends_with(b"\n") && !after.is_empty();

    let mut out =
        Vec::with_capacity(stripped_before.len() + usize::from(needs_line_sep) + after.len());
    out.extend_from_slice(stripped_before);
    if needs_line_sep {
        out.push(b'\n');
    }
    out.extend_from_slice(after);
    out
}

fn trim_trailing_ascii_spaces(mut line: &[u8]) -> &[u8] {
    while let Some((&last, rest)) = line.split_last() {
        if last == b' ' || last == b'\t' {
            line = rest;
        } else {
            break;
        }
    }
    line
}

fn quote_executable(shell: ShellKind, binary_override: Option<&Path>) -> String {
    let Some(path) = binary_override else {
        return format!("command {CLI_BIN}");
    };
    let raw = path.to_string_lossy();
    match shell {
        ShellKind::Zsh | ShellKind::Bash => {
            let escaped = raw.replace('\'', "'\\''");
            format!("command '{escaped}'")
        }
        ShellKind::Fish => {
            let mut escaped = String::with_capacity(raw.len());
            for ch in raw.chars() {
                match ch {
                    '\\' => escaped.push_str("\\\\"),
                    '\'' => escaped.push_str("\\'"),
                    _ => escaped.push(ch),
                }
            }
            format!("command '{escaped}'")
        }
    }
}

fn render_zsh_snippet(exec_cmd: &str) -> String {
    format!(
        r#"# Niwa opt-in shell signal hook (zsh) — records exit status and duration only.
autoload -Uz add-zsh-hook
typeset -g __niwa_cmd_start=""

__niwa_preexec() {{
  __niwa_cmd_start=$SECONDS
}}

__niwa_precmd() {{
  local __niwa_ec=$?
  emulate -L zsh
  setopt NO_BG_NICE NO_NOTIFY NO_MONITOR
  if [[ -n "${{__niwa_cmd_start:-}}" ]]; then
    local __niwa_dur=$(( SECONDS - __niwa_cmd_start ))
    __niwa_cmd_start=""
    if (( __niwa_dur < 0 )); then
      __niwa_dur=0
    fi
    ( {exec_cmd} _ingest --exit "$__niwa_ec" --dur "$__niwa_dur" >/dev/null 2>&1 & ) 2>/dev/null
  fi
  return $__niwa_ec
}}

add-zsh-hook -d precmd __niwa_precmd >/dev/null 2>&1
add-zsh-hook -d preexec __niwa_preexec >/dev/null 2>&1
add-zsh-hook precmd __niwa_precmd
add-zsh-hook preexec __niwa_preexec
precmd_functions=(__niwa_precmd ${{precmd_functions:#__niwa_precmd}})
"#
    )
}

fn render_bash_snippet(exec_cmd: &str) -> String {
    format!(
        r#"# Niwa opt-in shell signal hook (bash) — records exit status and duration only.
__niwa_cmd_start=""
__niwa_in_prompt=1

__niwa_preexec() {{
  [[ -n "${{COMP_LINE:-}}" ]] && return 0
  [[ -n "${{__niwa_in_prompt:-}}" ]] && return 0
  case "${{BASH_COMMAND:-}}" in
    __niwa_precmd*|__niwa_postprompt*) return 0 ;;
  esac
  if [[ -z "${{__niwa_cmd_start:-}}" ]]; then
    __niwa_cmd_start=$SECONDS
  fi
}}

__niwa_precmd() {{
  local __niwa_ec=$?
  __niwa_in_prompt=1
  if [[ -n "${{__niwa_cmd_start:-}}" ]]; then
    local __niwa_dur=$(( SECONDS - __niwa_cmd_start ))
    __niwa_cmd_start=""
    if (( __niwa_dur < 0 )); then
      __niwa_dur=0
    fi
    ( {exec_cmd} _ingest --exit "$__niwa_ec" --dur "$__niwa_dur" >/dev/null 2>&1 & ) 2>/dev/null
  fi
  return $__niwa_ec
}}

__niwa_postprompt() {{
  local __niwa_ec=$?
  __niwa_in_prompt=""
  return $__niwa_ec
}}

__niwa_install_bash() {{
  local __niwa_decl
  __niwa_decl="$(declare -p PROMPT_COMMAND 2>/dev/null || true)"
  if [[ "$__niwa_decl" == "declare -a"* ]]; then
    local __niwa_item __niwa_filtered=()
    for __niwa_item in "${{PROMPT_COMMAND[@]}}"; do
      if [[ "$__niwa_item" != "__niwa_precmd" && "$__niwa_item" != "__niwa_postprompt" ]]; then
        __niwa_filtered+=("$__niwa_item")
      fi
    done
    PROMPT_COMMAND=(__niwa_precmd "${{__niwa_filtered[@]}}" __niwa_postprompt)
  else
    case ";${{PROMPT_COMMAND:-}};" in
      *";__niwa_precmd;"*) ;;
      *)
        if [[ -z "${{PROMPT_COMMAND:-}}" ]]; then
          PROMPT_COMMAND="__niwa_precmd;__niwa_postprompt"
        else
          PROMPT_COMMAND="__niwa_precmd;${{PROMPT_COMMAND%;}};__niwa_postprompt"
        fi
        ;;
    esac
  fi
  trap '__niwa_preexec' DEBUG
}}
__niwa_install_bash
unset -f __niwa_install_bash
"#
    )
}

fn render_fish_snippet(exec_cmd: &str) -> String {
    format!(
        r#"# Niwa opt-in shell signal hook (fish) — records exit status and duration only.
function __niwa_postexec --on-event fish_postexec
    set -l __niwa_ec $status
    set -l __niwa_dur_ms $CMD_DURATION
    if test -z "$__niwa_dur_ms"
        set __niwa_dur_ms 0
    end
    set -l __niwa_dur (math "$__niwa_dur_ms / 1000")
    {exec_cmd} _ingest --exit "$__niwa_ec" --dur "$__niwa_dur" >/dev/null 2>&1 &
    set -l __niwa_pid $last_pid
    if test -n "$__niwa_pid"
        disown $__niwa_pid 2>/dev/null
    end
    return $__niwa_ec
end
"#
    )
}

fn resolve_home<F>(get_env: &mut F, fallback_home: Option<&Path>) -> Result<PathBuf, StoreError>
where
    F: FnMut(&str) -> Option<OsString>,
{
    non_empty_os(get_env("HOME"))
        .map(PathBuf::from)
        .or_else(|| fallback_home.map(Path::to_path_buf))
        .ok_or(StoreError::MissingHomeDirectory)
}

fn non_empty_os(val: Option<OsString>) -> Option<OsString> {
    val.filter(|v| !v.is_empty())
}

fn read_optional_bytes(path: &Path) -> Result<Option<Vec<u8>>, StoreError> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(StoreError::Io {
            path: path.to_path_buf(),
            action: "read shell rc file",
            source,
        }),
    }
}

fn remove_file_if_present(path: &Path, action: &'static str) -> Result<(), StoreError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(StoreError::Io {
            path: path.to_path_buf(),
            action,
            source,
        }),
    }
}

fn apply_permissions(path: &Path, perms: Option<&fs::Permissions>) {
    if let Some(perms) = perms {
        let _ = fs::set_permissions(path, perms.clone());
    }
}
