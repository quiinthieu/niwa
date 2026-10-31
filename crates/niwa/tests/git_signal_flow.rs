#[path = "../../../tests/support/mod.rs"]
mod support;

use assert_cmd::Command;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::time::Duration;
use support::TestEnv;
use tg_core::clock::{Clock, FakeClock, Timestamp};
use tg_core::product::CLI_BIN;
use tg_signals::activity::FakeIdleSource;
use tg_signals::git::{CountingGitLogRunner, ProcessGitLogRunner};
use tg_signals::{EventSpool, GitPollState, SignalKind};
use tg_store::config::ConfigStore;
use tg_store::save::GardenStore;

fn niwa_cmd(env: &TestEnv) -> Command {
    let mut cmd = Command::cargo_bin(CLI_BIN).expect("resolve compiled niwa binary");
    for (key, value) in env.env_vars() {
        cmd.env(key, value);
    }
    cmd.env("NO_COLOR", "1");
    cmd
}

fn init_temp_repo(home: &Path, relative_dir: &str, branch: &str) -> PathBuf {
    let repo_dir = home.join(relative_dir);
    fs::create_dir_all(&repo_dir).expect("create temp git repo dir");
    for args in [&["init", "-q"][..], &["checkout", "-q", "-B", branch][..]] {
        let out = ProcessCommand::new("git")
            .arg("-C")
            .arg(&repo_dir)
            .args(args)
            .env("HOME", home)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .expect("invoke git init/checkout");
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }
    repo_dir
}

#[allow(clippy::too_many_arguments)]
fn commit_in_repo(
    home: &Path,
    repo_dir: &Path,
    file_name: &str,
    contents: &str,
    message: &str,
    author_name: &str,
    author_email: &str,
    ts: Timestamp,
) -> String {
    fs::write(repo_dir.join(file_name), contents).expect("write test file");
    let add_out = ProcessCommand::new("git")
        .arg("-C")
        .arg(repo_dir)
        .args(["add", file_name])
        .env("HOME", home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .expect("git add");
    assert!(add_out.status.success());

    let date = format!("@{}", ts.as_unix_secs());
    let commit_out = ProcessCommand::new("git")
        .arg("-C")
        .arg(repo_dir)
        .args(["-c", "commit.gpgsign=false", "commit", "-q", "-m", message])
        .env("HOME", home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_AUTHOR_NAME", author_name)
        .env("GIT_AUTHOR_EMAIL", author_email)
        .env("GIT_AUTHOR_DATE", &date)
        .env("GIT_COMMITTER_NAME", author_name)
        .env("GIT_COMMITTER_EMAIL", author_email)
        .env("GIT_COMMITTER_DATE", &date)
        .output()
        .expect("git commit");
    assert!(
        commit_out.status.success(),
        "git commit failed: {}",
        String::from_utf8_lossy(&commit_out.stderr)
    );

    let sha_out = ProcessCommand::new("git")
        .arg("-C")
        .arg(repo_dir)
        .args(["rev-parse", "HEAD"])
        .env("HOME", home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .expect("git rev-parse HEAD");
    assert!(sha_out.status.success());
    String::from_utf8(sha_out.stdout)
        .unwrap()
        .trim()
        .to_string()
}

#[test]
fn cli_and_daemon_git_signal_awards_water_throttles_every_5_min_and_purges_cleanly() {
    let git_env = TestEnv::new();
    let control_env = TestEnv::new();
    let t0_secs = 1_700_050_000_i64;
    let git_clock = FakeClock::from_unix_secs(t0_secs);
    let control_clock = FakeClock::from_unix_secs(t0_secs);

    let repo_dir = init_temp_repo(git_env.home_dir(), "work/garden-app", "main");

    // Configure `git.repos` and `git.author_email` through the real `niwa config set` CLI.
    niwa_cmd(&git_env)
        .args([
            "config",
            "set",
            "git.repos",
            repo_dir.to_str().expect("utf-8 repo path"),
        ])
        .assert()
        .success();
    niwa_cmd(&git_env)
        .args(["config", "set", "git.author_email", "<alice@example.test>"])
        .assert()
        .success();

    // Verify normalized email round-trips through `niwa config get`.
    let email_out = niwa_cmd(&git_env)
        .args(["config", "get", "git.author_email"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(
        String::from_utf8(email_out).unwrap().trim(),
        "alice@example.test"
    );

    // Stamp deterministic `consented_at = t0` in `git_env` so `FakeClock` simulation aligns at `t0`.
    {
        let cfg_store = ConfigStore::new(git_env.paths().clone());
        let mut cfg = cfg_store.load().unwrap();
        cfg.enable_signal(SignalKind::Git, git_clock.now());
        cfg_store.save(&cfg).unwrap();
    }

    // Initialize both gardens with identical seed and plant the same starter seed in plot 0.
    {
        let mut s_git = niwa_garden::runtime::GardenSession::open(
            git_env.paths().clone(),
            &git_clock,
            Some(2026),
        )
        .unwrap();
        let starter_id = s_git.save().inventory.keys().next().unwrap().clone();
        s_git.plant(0, &starter_id, git_clock.now()).unwrap();

        let mut s_ctrl = niwa_garden::runtime::GardenSession::open(
            control_env.paths().clone(),
            &control_clock,
            Some(2026),
        )
        .unwrap();
        s_ctrl.plant(0, &starter_id, control_clock.now()).unwrap();
    }

    // `GardenSession::open` above already performed the initial catch-up poll at `t0`
    // (`last_poll_at = t0`). Running `niwa daemon` across 10 sixty-second wakes (`t0 + 60`
    // through `t0 + 600`, 10 minutes total) must throttle periodic git polling to once every
    // 5 minutes (`300 s`): wake #5 (`t0 + 300`) and wake #10 (`t0 + 600`) -> 2 polls total.
    // After wake #1 (`t0 + 60`), Alice commits at `t0 + 90` (inside the throttled window);
    // wake #5 (`t0 + 300`) discovers that `t0 + 90` commit and awards its full water GP!
    git_clock.advance_ticks(1);
    let runner = CountingGitLogRunner::new(ProcessGitLogRunner::with_home(git_env.home_dir()));
    let idle = FakeIdleSource::unavailable();

    let summary = niwa_garden::daemon::run_daemon_loop_with_git(
        git_env.paths().clone(),
        &git_clock,
        &idle,
        &runner,
        &|dur: Duration| {
            let now_before_sleep = git_clock.now();
            if now_before_sleep == Timestamp::from_unix_secs(t0_secs + 60) {
                let _ = commit_in_repo(
                    git_env.home_dir(),
                    &repo_dir,
                    "feature.rs",
                    "fn grow() {}\n",
                    "feat: add grow helper",
                    "Alice",
                    "alice@example.test",
                    Timestamp::from_unix_secs(t0_secs + 90),
                );
            }
            git_clock.advance_secs(i64::try_from(dur.as_secs()).unwrap());
        },
        Some(10),
    )
    .expect("run daemon loop with git signal");

    assert_eq!(summary.wakes_executed, 10);
    assert_eq!(summary.ticks_simulated, 10);
    assert_eq!(
        idle.calls(),
        0,
        "activity is disabled so IdleSource must not run"
    );
    assert_eq!(
        runner.calls(),
        2,
        "60 s daemon loop over 10 minutes after t0 open must poll git twice (at t0+300 and t0+600)"
    );

    control_clock.advance_ticks(10);
    let s_git_after =
        niwa_garden::runtime::GardenSession::open(git_env.paths().clone(), &git_clock, None)
            .unwrap();
    let s_ctrl_after = niwa_garden::runtime::GardenSession::open(
        control_env.paths().clone(),
        &control_clock,
        None,
    )
    .unwrap();

    let git_gp = s_git_after.garden().plots()[0].plant().unwrap().total_gp;
    let ctrl_gp = s_ctrl_after.garden().plots()[0].plant().unwrap().total_gp;
    assert!(
        git_gp > ctrl_gp,
        "consented git commit must grant water GP bonus ({git_gp} > {ctrl_gp})"
    );
    assert_eq!(s_git_after.save().git_poll.credited_commits, 1);

    // Queue one partial-tick commit (`t0 + 620` at `now = t0 + 630`) and run `niwa signals disable git --purge`:
    // verifies both `events.jsonl` and `garden.json` `git_poll` are purged cleanly.
    let _ = commit_in_repo(
        git_env.home_dir(),
        &repo_dir,
        "second.rs",
        "fn bloom() {}\n",
        "feat: add bloom helper",
        "Alice",
        "alice@example.test",
        Timestamp::from_unix_secs(t0_secs + 620),
    );
    git_clock.advance_secs(30);
    let _ = niwa_garden::runtime::GardenSession::open(git_env.paths().clone(), &git_clock, None)
        .unwrap();
    assert_eq!(
        EventSpool::new(git_env.paths().clone())
            .read()
            .unwrap()
            .events
            .len(),
        1
    );

    niwa_cmd(&git_env)
        .args(["signals", "disable", "git", "--purge"])
        .assert()
        .success();

    let cfg_after_purge = ConfigStore::new(git_env.paths().clone()).load().unwrap();
    assert!(!cfg_after_purge.signals.git.enabled);
    assert_eq!(cfg_after_purge.signals.git.consented_at, None);
    assert!(!git_env.paths().events_file().exists());

    let loaded_save = GardenStore::new(git_env.paths().clone())
        .load()
        .unwrap()
        .unwrap()
        .save;
    assert_eq!(loaded_save.git_poll, GitPollState::default());
}

#[test]
fn privacy_canary_repo_paths_messages_hashes_and_emails_never_appear_in_save_or_spool() {
    let env = TestEnv::new();
    let t0 = Timestamp::from_unix_secs(1_700_100_000);
    let clock = FakeClock::new(t0);

    let repo_dir = init_temp_repo(
        env.home_dir(),
        "ultra_secret_project_canary_9911/stealth_repo_canary_8822",
        "feature/top_secret_branch_canary_7733",
    );
    let author_name = "Secret Gardener Canary";
    let author_email = "canary_author_3333@private-domain.example";

    // Enable `git` at `t0` and initialize garden save.
    {
        let cfg_store = ConfigStore::new(env.paths().clone());
        let mut cfg = cfg_store.load().unwrap();
        cfg.git.repos = vec![repo_dir.clone()];
        cfg.git.author_email = Some(author_email.to_string());
        cfg.enable_signal(SignalKind::Git, t0);
        cfg_store.save(&cfg).unwrap();
    }
    let _ =
        niwa_garden::runtime::GardenSession::open(env.paths().clone(), &clock, Some(77)).unwrap();

    // Create commit #1 at `t0 + 20`, then amend its message at `t0 + 25`.
    let sha_orig = commit_in_repo(
        env.home_dir(),
        &repo_dir,
        "classified_formula_canary_6644.rs",
        "pub const SECRET_CANARY_PAYLOAD: &str = \"LAUNCH_CODE_CANARY_2222\";\n",
        "TOP_SECRET_COMMIT_MSG_CANARY_5555: launch codes",
        author_name,
        author_email,
        t0.add_secs(20),
    );
    let amend_date = format!("@{}", t0.add_secs(25).as_unix_secs());
    let amend_out = ProcessCommand::new("git")
        .arg("-C")
        .arg(&repo_dir)
        .args([
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--amend",
            "-q",
            "-m",
            "AMENDED_SECRET_MSG_CANARY_4444: rotated codes",
        ])
        .env("HOME", env.home_dir())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_COMMITTER_NAME", author_name)
        .env("GIT_COMMITTER_EMAIL", author_email)
        .env("GIT_COMMITTER_DATE", &amend_date)
        .output()
        .expect("amend canary commit");
    assert!(amend_out.status.success());

    let sha_amended = String::from_utf8(
        ProcessCommand::new("git")
            .arg("-C")
            .arg(&repo_dir)
            .args(["rev-parse", "HEAD"])
            .env("HOME", env.home_dir())
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_string();

    // 1. Open at `t0 + 30` (partial tick `< 60 s`) so `events.jsonl` stays uncompacted on disk
    //    with the live `git_commit` line while `garden.json`, `garden.json.bak`, and `snapshot.json`
    //    are all written!
    clock.advance_secs(30);
    let _ = niwa_garden::runtime::GardenSession::open(env.paths().clone(), &clock, None).unwrap();
    assert!(
        env.paths().events_file().is_file(),
        "partial-tick git_commit must be present on disk in events.jsonl for privacy inspection"
    );

    // Validate exact schema of every line in `events.jsonl`: ONLY `{"kind": "git_commit", "ts": <i64>}`.
    let raw_spool_before_compact = fs::read_to_string(env.paths().events_file()).unwrap();
    for line in raw_spool_before_compact.lines() {
        let obj: Value = serde_json::from_str(line).expect("valid JSON line in spool");
        let map = obj.as_object().expect("JSON object");
        assert_eq!(
            map.len(),
            2,
            "git_commit spool event must have strictly 2 fields: {line}"
        );
        assert_eq!(map.get("kind").and_then(Value::as_str), Some("git_commit"));
        assert!(map.get("ts").and_then(Value::as_i64).is_some());
    }

    // 2. Advance past tick 1 (`t0 + 60`) to apply commit #1, then add commit #2 at `t0 + 75`
    //    and open at `t0 + 80` so `garden.json`, `garden.json.bak`, `snapshot.json`, AND `events.jsonl`
    //    all exist simultaneously after a full apply + backup cycle.
    clock.advance_secs(30);
    let _ = niwa_garden::runtime::GardenSession::open(env.paths().clone(), &clock, None).unwrap();

    let sha_second = commit_in_repo(
        env.home_dir(),
        &repo_dir,
        "classified_formula_canary_6644.rs",
        "pub const SECRET_CANARY_PAYLOAD: &str = \"ROTATED_CANARY_1111\";\n",
        "SECOND_SECRET_COMMIT_MSG_CANARY_0000",
        author_name,
        author_email,
        t0.add_secs(75),
    );
    clock.advance_secs(20);
    let _ = niwa_garden::runtime::GardenSession::open(env.paths().clone(), &clock, None).unwrap();

    let inspected_files = [
        env.paths().garden_file(),
        env.paths().garden_backup_file(),
        env.paths().snapshot_file(),
        env.paths().events_file(),
    ];
    for path in &inspected_files {
        assert!(
            path.is_file(),
            "expected state file {} to exist for privacy inspection",
            path.display()
        );
    }

    let forbidden_canaries = [
        repo_dir.to_str().unwrap(),
        "ultra_secret_project_canary_9911",
        "stealth_repo_canary_8822",
        "top_secret_branch_canary_7733",
        "classified_formula_canary_6644",
        "LAUNCH_CODE_CANARY_2222",
        "ROTATED_CANARY_1111",
        "TOP_SECRET_COMMIT_MSG_CANARY_5555",
        "AMENDED_SECRET_MSG_CANARY_4444",
        "SECOND_SECRET_COMMIT_MSG_CANARY_0000",
        "launch codes",
        "rotated codes",
        author_name,
        author_email,
        "canary_author_3333",
        "private-domain.example",
        sha_orig.as_str(),
        &sha_orig[..8],
        sha_amended.as_str(),
        &sha_amended[..8],
        sha_second.as_str(),
        &sha_second[..8],
    ];

    for file_path in &inspected_files {
        let contents = fs::read_to_string(file_path).unwrap();
        let lower = contents.to_ascii_lowercase();
        for canary in forbidden_canaries {
            let canary_lower = canary.to_ascii_lowercase();
            assert!(
                !lower.contains(&canary_lower),
                "PRIVACY VIOLATION: canary {:?} leaked into {}",
                canary,
                file_path.display()
            );
        }
    }
}
