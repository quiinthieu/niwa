#[path = "../../../tests/support/mod.rs"]
mod support;

use assert_cmd::Command;
use clap::Parser;
use niwa_garden::cli::{execute_with, Cli};
use serde_json::Value;
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command as StdCommand, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use support::TestEnv;
use tg_core::clock::{FakeClock, Timestamp};
use tg_store::config::{ConfigStore, SignalKind};
use tg_store::lock::GardenLock;
use tg_store::shell_hook::{backup_path_for_rc, ShellKind, HOOK_BLOCK_END, HOOK_BLOCK_START};
use tg_store::spool::{DurationBucket, EventSpool, ShellExitClass, WorkEvent};

fn niwa_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_niwa"))
}

fn niwa_cmd(env: &TestEnv) -> Command {
    let mut cmd = Command::new(niwa_bin());
    for (k, v) in env.env_vars() {
        cmd.env(k, v);
    }
    cmd.env_remove("ZDOTDIR");
    cmd
}

fn find_program_in_path(program: &str) -> Option<PathBuf> {
    let path_var = env::var_os("PATH")?;
    for dir in env::split_paths(&path_var) {
        let candidate = dir.join(program);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

#[test]
fn cli_hook_prints_zsh_bash_and_fish_snippets() {
    let env = TestEnv::new();

    for shell in ["zsh", "bash", "fish"] {
        let output = niwa_cmd(&env)
            .args(["hook", shell])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let text = String::from_utf8(output).expect("hook snippet is UTF-8");
        assert!(
            text.contains("_ingest --exit") && text.contains("--dur"),
            "{shell} snippet must background _ingest:\n{text}"
        );
    }
}

#[test]
fn cli_hook_install_dry_run_vs_write_rc_and_uninstall_round_trip_byte_for_byte() {
    let env = TestEnv::new();
    let zshrc = env.home_dir().join(".zshrc");
    let bak = backup_path_for_rc(&zshrc);
    let original = b"# my zshrc\nexport LANG=en_US.UTF-8\nalias g='git status'";
    fs::write(&zshrc, original).expect("seed .zshrc without trailing newline");

    // Without `--write-rc`, `niwa hook install` previews the block and never edits `.zshrc`.
    let preview_out = niwa_cmd(&env)
        .env("SHELL", "/bin/zsh")
        .args(["hook", "install"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let preview_text = String::from_utf8(preview_out).expect("UTF-8 preview");
    assert!(preview_text.contains(HOOK_BLOCK_START));
    assert!(preview_text.contains("--write-rc"));
    assert_eq!(fs::read(&zshrc).expect("zshrc untouched"), original);
    assert!(!bak.exists());

    // With `--write-rc`, `niwa hook install` writes `.zshrc.niwa.bak` and adds the marked block.
    niwa_cmd(&env)
        .env("SHELL", "/bin/zsh")
        .args(["hook", "install", "--write-rc"])
        .assert()
        .success();

    let hooked = fs::read_to_string(&zshrc).expect("read hooked .zshrc");
    assert!(hooked.contains(HOOK_BLOCK_START));
    assert!(hooked.contains(HOOK_BLOCK_END));
    assert_eq!(fs::read(&bak).expect("backup matches original"), original);

    // Bare `niwa hook uninstall` removes the marked block and restores `.zshrc` byte for byte.
    niwa_cmd(&env)
        .env("SHELL", "/bin/zsh")
        .args(["hook", "uninstall"])
        .assert()
        .success();

    assert_eq!(
        fs::read(&zshrc).expect("restored .zshrc"),
        original,
        "CLI hook install --write-rc -> hook uninstall must restore rc byte for byte"
    );
    assert!(!bak.exists());
}

#[test]
fn ingest_enforces_consent_classifies_buckets_and_never_waits_on_garden_lock() {
    let env = TestEnv::new();
    let spool = EventSpool::new(env.paths().clone());
    let t0 = Timestamp::from_unix_secs(1_700_100_000);
    let clock = FakeClock::new(t0);

    // 1. Before `signals enable shell`, `_ingest` is a silent no-op and writes nothing.
    let cli_before =
        Cli::try_parse_from(["niwa", "_ingest", "--exit", "0", "--dur", "0.2"]).expect("parse");
    execute_with(cli_before, env.paths().clone(), &clock).expect("_ingest disabled no-op");
    assert!(!env.paths().events_file().exists());

    // Enable `shell` signal at `t0`.
    let config_store = ConfigStore::new(env.paths().clone());
    let mut cfg = config_store.load().expect("load default config");
    cfg.enable_signal(SignalKind::Shell, t0);
    config_store.save(&cfg).expect("save enabled shell config");

    // 2. Acquire `GardenLock` in this thread and invoke a real `niwa _ingest` subprocess to prove
    // `_ingest` appends without waiting on `garden.lock` (PLAN §5).
    let held_lock = GardenLock::acquire(env.paths()).expect("hold garden.lock");
    niwa_cmd(&env)
        .args(["_ingest", "--exit", "0", "--dur", "0.15"])
        .assert()
        .success();
    drop(held_lock);

    // Clear that wall-clock event so we can verify exact deterministic classifications via FakeClock.
    fs::remove_file(env.paths().events_file()).expect("clear wall-clock spool line");

    // 3. Verify exit-code and duration-bucket classification (including negative exit codes and
    // European comma-decimal durations like `12,5`).
    let samples = [
        ("0", "0.05", ShellExitClass::Ok, DurationBucket::Instant),
        ("1", "2.5", ShellExitClass::Err, DurationBucket::Fast),
        ("-2", "12,5", ShellExitClass::Err, DurationBucket::Medium),
        ("127", "75", ShellExitClass::Err, DurationBucket::Long),
    ];

    for (idx, (code, dur, expected_status, expected_bucket)) in samples.into_iter().enumerate() {
        let ts = Timestamp::from_unix_secs(t0.as_unix_secs() + (idx as i64) + 1);
        clock.set(ts);
        let parsed =
            Cli::try_parse_from(["niwa", "_ingest", "--exit", code, "--dur", dur]).expect("parse");
        execute_with(parsed, env.paths().clone(), &clock).expect("execute _ingest");

        let events = spool.read().expect("read spool").events;
        assert_eq!(
            events.last().copied(),
            Some(WorkEvent::Shell {
                ts,
                status: expected_status,
                dur_bucket: expected_bucket,
            })
        );
    }
}

fn wait_for_spool_events(spool: &EventSpool, min_count: usize) -> Vec<WorkEvent> {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let outcome = spool.read().expect("read spool");
        if outcome.events.len() >= min_count || Instant::now() >= deadline {
            // Brief settle pause to catch any spurious trailing background events.
            thread::sleep(Duration::from_millis(60));
            return spool.read().expect("re-read settled spool").events;
        }
        thread::sleep(Duration::from_millis(15));
    }
}

fn run_posix_shell_piped(
    env: &TestEnv,
    shell: ShellKind,
    shell_binary: &Path,
    rc_path: &Path,
    secret_cmd_token: &str,
    secret_path_token: &str,
) -> (bool, String, String) {
    let mut child_cmd = StdCommand::new(shell_binary);
    child_cmd.env_clear();
    if let Some(path_val) = env::var_os("PATH") {
        child_cmd.env("PATH", path_val);
    }
    for (k, v) in env.env_vars() {
        child_cmd.env(k, v);
    }
    child_cmd
        .env("TERM", "dumb")
        .env("XDG_DATA_HOME", env.xdg_state_home())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    match shell {
        ShellKind::Zsh => {
            child_cmd.env("ZDOTDIR", env.home_dir());
            child_cmd.args(["+Z", "-d", "-i"]);
        }
        ShellKind::Bash => {
            child_cmd.arg("--noprofile");
            child_cmd.arg("--rcfile");
            child_cmd.arg(rc_path);
            child_cmd.arg("-i");
        }
        ShellKind::Fish => unreachable!("fish uses PTY runner"),
    }

    let mut child = child_cmd
        .spawn()
        .unwrap_or_else(|err| panic!("failed to spawn real {shell} at {shell_binary:?}: {err}"));

    let script = format!(
        "\necho {secret_cmd_token} {secret_path_token} >/dev/null\nsh -c 'exit 7'\nsleep 1\n\nexit 0\n"
    );

    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(script.as_bytes())
        .expect("write interactive script to shell stdin");

    let output = child
        .wait_with_output()
        .expect("wait for interactive shell");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// Drives `fish -i` inside a real controlling PTY (`pty.fork()` -> `setsid` + `TIOCSCTTY`)
/// synchronized turn-by-turn on `PROMPT_EC=` so Fish uses its interactive line reader
/// (`fish_postexec` + `fish_prompt`) on both Linux and macOS CI runners.
fn run_fish_in_pty(
    env: &TestEnv,
    fish_binary: &Path,
    secret_cmd_token: &str,
    secret_path_token: &str,
) -> (bool, String, String) {
    let py_driver = r#"
import os, pty, select, signal, sys, time

fish_bin, secret_cmd, secret_path = sys.argv[1], sys.argv[2], sys.argv[3]
pid, master = pty.fork()
if pid == 0:
    os.execv(fish_bin, [fish_bin, "-i"])

transcript = bytearray()

def drain_quiet(window_secs=0.03):
    end = time.monotonic() + window_secs
    while True:
        rem = end - time.monotonic()
        if rem <= 0:
            break
        ready, _, _ = select.select([master], [], [], rem)
        if not ready:
            break
        try:
            chunk = os.read(master, 4096)
        except OSError:
            break
        if not chunk:
            break
        transcript.extend(chunk)

def wait_for_prompt_since(cursor, step_label):
    deadline = time.monotonic() + 15.0
    while b"PROMPT_EC=" not in transcript[cursor:]:
        rem = deadline - time.monotonic()
        if rem <= 0:
            sys.stderr.write(f"Timed out waiting for fish prompt ({step_label})\n")
            sys.stdout.buffer.write(bytes(transcript))
            try:
                os.kill(pid, signal.SIGKILL)
            except OSError:
                pass
            sys.exit(99)
        ready, _, _ = select.select([master], [], [], min(rem, 0.25))
        if not ready:
            wpid, wst = os.waitpid(pid, os.WNOHANG)
            if wpid == pid:
                ec = os.waitstatus_to_exitcode(wst)
                sys.stderr.write(f"fish exited early ({ec}) during {step_label}\n")
                sys.stdout.buffer.write(bytes(transcript))
                sys.exit(ec if ec != 0 else 98)
            continue
        try:
            chunk = os.read(master, 4096)
        except OSError as exc:
            sys.stderr.write(f"PTY OSError during {step_label}: {exc}\n")
            sys.stdout.buffer.write(bytes(transcript))
            _, wst = os.waitpid(pid, 0)
            ec = os.waitstatus_to_exitcode(wst)
            sys.exit(ec if ec != 0 else 97)
        if not chunk:
            sys.stderr.write(f"PTY EOF during {step_label}\n")
            sys.stdout.buffer.write(bytes(transcript))
            _, wst = os.waitpid(pid, 0)
            ec = os.waitstatus_to_exitcode(wst)
            sys.exit(ec if ec != 0 else 96)
        transcript.extend(chunk)
    drain_quiet()

wait_for_prompt_since(0, "initial_prompt")

commands = [
    ("blank_1", "\r"),
    ("secret_echo", f"echo {secret_cmd} {secret_path} >/dev/null\r"),
    ("fail_7", "sh -c 'exit 7'\r"),
    ("sleep_1", "sleep 1\r"),
    ("blank_2", "\r"),
]

for label, line in commands:
    cursor = len(transcript)
    os.write(master, line.encode("utf-8"))
    wait_for_prompt_since(cursor, label)

os.write(master, b"exit 0\r")
end_deadline = time.monotonic() + 10.0
exit_code = None
while time.monotonic() < end_deadline:
    ready, _, _ = select.select([master], [], [], 0.2)
    if ready:
        try:
            chunk = os.read(master, 4096)
            if chunk:
                transcript.extend(chunk)
                continue
        except OSError:
            pass
    wpid, wst = os.waitpid(pid, os.WNOHANG)
    if wpid == pid:
        exit_code = os.waitstatus_to_exitcode(wst)
        break

if exit_code is None:
    try:
        os.kill(pid, signal.SIGKILL)
    except OSError:
        pass
    _, wst = os.waitpid(pid, 0)
    exit_code = os.waitstatus_to_exitcode(wst)

os.close(master)
sys.stdout.buffer.write(bytes(transcript))
sys.exit(exit_code)
"#;

    let mut cmd = StdCommand::new("python3");
    cmd.env_clear();
    if let Some(path_val) = env::var_os("PATH") {
        cmd.env("PATH", path_val);
    }
    for (k, v) in env.env_vars() {
        cmd.env(k, v);
    }
    cmd.env("TERM", "dumb")
        .env("XDG_DATA_HOME", env.xdg_state_home())
        .arg("-c")
        .arg(py_driver)
        .arg(fish_binary)
        .arg(secret_cmd_token)
        .arg(secret_path_token);

    let output = cmd.output().expect("spawn python3 PTY driver for fish");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn verify_real_interactive_shell(shell: ShellKind, shell_binary: &Path) {
    let env = TestEnv::new();

    // Enable the opt-in `shell` signal via the real CLI.
    niwa_cmd(&env)
        .args(["signals", "enable", "shell", "--yes"])
        .assert()
        .success();

    let (rc_path, pre_existing_rc): (PathBuf, &[u8]) = match shell {
        ShellKind::Zsh => (
            env.home_dir().join(".zshrc"),
            b"unsetopt PROMPT_SP\nPS1='PROMPT_EC=%?\n'\nexport NIWA_USER_RC_LOADED=1\n",
        ),
        ShellKind::Bash => (
            env.home_dir().join(".bashrc"),
            b"PS1='PROMPT_EC=$?\n'\nexport NIWA_USER_RC_LOADED=1\n",
        ),
        ShellKind::Fish => (
            env.xdg_config_home().join("fish").join("config.fish"),
            b"function fish_greeting\nend\nfunction fish_prompt\n    echo \"PROMPT_EC=$status\"\nend\nset -gx NIWA_USER_RC_LOADED 1\n",
        ),
    };

    if let Some(parent) = rc_path.parent() {
        fs::create_dir_all(parent).expect("create rc parent dir");
    }
    fs::write(&rc_path, pre_existing_rc).expect("seed pre-existing rc");

    // Install the hook pointing directly at the compiled `niwa` test binary.
    niwa_cmd(&env)
        .args([
            "hook",
            "install",
            "--shell",
            shell.as_str(),
            "--bin",
            &niwa_bin().to_string_lossy(),
            "--write-rc",
        ])
        .assert()
        .success();

    let secret_cmd_token = "NIWA_CANARY_CMD_TEXT_9941";
    let secret_path_token = "/tmp/niwa_canary_secret_dir_9941/secret_file.txt";

    let (status_ok, stdout_str, stderr_str) = if shell == ShellKind::Fish {
        run_fish_in_pty(&env, shell_binary, secret_cmd_token, secret_path_token)
    } else {
        run_posix_shell_piped(
            &env,
            shell,
            shell_binary,
            &rc_path,
            secret_cmd_token,
            secret_path_token,
        )
    };

    assert!(
        status_ok,
        "real {shell} exited non-zero\nstdout:\n{stdout_str}\nstderr:\n{stderr_str}"
    );
    assert!(
        !stderr_str.contains("nice(5) failed"),
        "real {shell} emitted background nice warning:\n{stderr_str}"
    );
    assert!(
        stdout_str.contains("PROMPT_EC=7") || stderr_str.contains("PROMPT_EC=7"),
        "real {shell} prompt did not observe preserved exit status 7:\nstdout:\n{stdout_str}\nstderr:\n{stderr_str}"
    );

    let spool = EventSpool::new(env.paths().clone());
    let events = wait_for_spool_events(&spool, 3);
    assert_eq!(
        events.len(),
        3,
        "expected exactly 3 shell events (blank lines and startup must emit 0) for {shell}, got {events:?}"
    );

    let mut pairs: Vec<(ShellExitClass, DurationBucket)> = events
        .iter()
        .map(|ev| match *ev {
            WorkEvent::Shell {
                status, dur_bucket, ..
            } => (status, dur_bucket),
            other => panic!("unexpected non-shell event from {shell}: {other:?}"),
        })
        .collect();
    pairs.sort_by_key(|(s, b)| (s.as_str(), b.as_str()));

    let mut expected_pairs = vec![
        (ShellExitClass::Ok, DurationBucket::Instant),
        (ShellExitClass::Err, DurationBucket::Instant),
        (ShellExitClass::Ok, DurationBucket::Fast),
    ];
    expected_pairs.sort_by_key(|(s, b)| (s.as_str(), b.as_str()));
    assert_eq!(
        pairs, expected_pairs,
        "unexpected classified shell events for {shell}"
    );

    // Inspect raw JSONL lines in `events.jsonl` to prove ONLY `{ts, kind, status, dur_bucket}` exist.
    let raw_spool = fs::read_to_string(env.paths().events_file()).expect("read raw events.jsonl");
    let raw_lines: Vec<&str> = raw_spool.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(raw_lines.len(), 3);
    for line in &raw_lines {
        let val: Value = serde_json::from_str(line).expect("parse spool JSON line");
        let obj = val.as_object().expect("JSON object");
        let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec!["dur_bucket", "kind", "status", "ts"],
            "spool line for {shell} must contain timestamp, exit class and duration bucket only: {line}"
        );
        assert_eq!(obj["kind"], "shell");
    }

    // Trigger catch-up & save (`niwa status --json`) and run privacy grep across BOTH spool and save!
    let home_leak = env.home_dir().to_string_lossy().to_string();
    for forbidden in [
        secret_cmd_token,
        secret_path_token,
        "niwa_canary_secret_dir_9941",
        "sh -c",
        "sleep 1",
        home_leak.as_str(),
    ] {
        assert!(
            !raw_spool.contains(forbidden),
            "privacy leak in events.jsonl for {shell}: found '{forbidden}'"
        );
    }

    niwa_cmd(&env).args(["status", "--json"]).assert().success();

    let raw_save = fs::read_to_string(env.paths().garden_file()).expect("read garden.json");
    for forbidden in [
        secret_cmd_token,
        secret_path_token,
        "niwa_canary_secret_dir_9941",
        "sh -c",
        "sleep 1",
        home_leak.as_str(),
    ] {
        assert!(
            !raw_save.contains(forbidden),
            "privacy leak in garden.json for {shell}: found '{forbidden}'"
        );
    }

    // Finally, verify `niwa hook uninstall` restores the pre-existing rc file byte for byte.
    niwa_cmd(&env)
        .args(["hook", "uninstall", "--shell", shell.as_str()])
        .assert()
        .success();
    assert_eq!(
        fs::read(&rc_path).expect("read uninstalled rc"),
        pre_existing_rc,
        "rc file for {shell} was not restored byte for byte after real-shell session"
    );
}

#[test]
fn real_shells_produce_privacy_safe_spool_lines_and_preserve_exit_codes() {
    let zsh_bin = find_program_in_path("zsh").expect("zsh must be installed");
    verify_real_interactive_shell(ShellKind::Zsh, &zsh_bin);

    let bash_bin = find_program_in_path("bash").expect("bash must be installed");
    verify_real_interactive_shell(ShellKind::Bash, &bash_bin);

    match find_program_in_path("fish") {
        Some(fish_bin) => verify_real_interactive_shell(ShellKind::Fish, &fish_bin),
        None => {
            assert!(
                env::var_os("CI").is_none() && env::var_os("GITHUB_ACTIONS").is_none(),
                "fish shell must be installed in CI to verify real fish_postexec hook (T12 AC 1)"
            );
        }
    }
}

#[test]
fn ingest_subprocess_fast_path_stays_responsive() {
    let env = TestEnv::new();
    niwa_cmd(&env)
        .args(["signals", "enable", "shell", "--yes"])
        .assert()
        .success();

    // Warm-up run.
    let status = StdCommand::new(niwa_bin())
        .envs(env.env_vars())
        .args(["_ingest", "--exit", "0", "--dur", "0.4"])
        .status()
        .expect("warm-up _ingest");
    assert!(status.success());

    let samples = 20_usize;
    let mut durations_ms = Vec::with_capacity(samples);
    for _ in 0..samples {
        let start = Instant::now();
        let st = StdCommand::new(niwa_bin())
            .envs(env.env_vars())
            .args(["_ingest", "--exit", "0", "--dur", "0.4"])
            .status()
            .expect("spawn _ingest");
        assert!(st.success());
        durations_ms.push(start.elapsed().as_secs_f64() * 1000.0);
    }

    durations_ms.sort_by(|a, b| a.total_cmp(b));
    let p95_idx = ((samples * 95).div_ceil(100)).saturating_sub(1);
    let p95_ms = durations_ms[p95_idx];

    // Debug-mode CI runners can experience scheduling jitter; release-mode p95 (< 10 ms)
    // is measured and recorded in `docs/perf.md`.
    assert!(
        p95_ms < 250.0,
        "unexpectedly slow _ingest subprocess p95 latency: {p95_ms:.2} ms"
    );
}
