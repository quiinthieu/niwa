#[path = "../../../tests/support/mod.rs"]
mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use support::TestEnv;
use tg_core::clock::Timestamp;
use tg_signals::git::{
    expand_repo_path, parse_git_log_output, poll_and_record_git_commits, CountingGitLogRunner,
    FakeGitLogRunner, GitPollMode, GitPollOutcome, ObservedGitCommit, ProcessGitLogRunner,
    GIT_POLL_INTERVAL_SECS,
};
use tg_signals::{EventSpool, GitPollState, SignalKind, WorkEvent};
use tg_store::config::AppConfig;

struct TempGitRepo {
    path: PathBuf,
    home: PathBuf,
}

impl TempGitRepo {
    fn init(env: &TestEnv, name: &str) -> Self {
        let path = env.home_dir().join("repos").join(name);
        fs::create_dir_all(&path).expect("create temp repo directory");
        let repo = Self {
            path,
            home: env.home_dir().to_path_buf(),
        };
        repo.git(&["init", "-q"]);
        repo.git(&["checkout", "-q", "-B", "main"]);
        repo
    }

    fn clone_from(env: &TestEnv, source: &Path, clone_name: &str) -> Self {
        let dest = env.home_dir().join("repos").join(clone_name);
        fs::create_dir_all(dest.parent().unwrap()).unwrap();
        let status = Command::new("git")
            .args([
                "clone",
                "-q",
                source.to_str().expect("utf-8 source path"),
                dest.to_str().expect("utf-8 dest path"),
            ])
            .env("HOME", env.home_dir())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .status()
            .expect("invoke git clone");
        assert!(status.success(), "git clone failed");
        Self {
            path: dest,
            home: env.home_dir().to_path_buf(),
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn git(&self, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.path)
            .args(args)
            .env("HOME", &self.home)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .expect("invoke git in temp repo");
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_stdout(&self, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.path)
            .args(args)
            .env("HOME", &self.home)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .expect("invoke git in temp repo");
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).expect("utf-8 git stdout")
    }

    #[allow(clippy::too_many_arguments)]
    fn commit_at(
        &self,
        file_name: &str,
        contents: &str,
        message: &str,
        author_name: &str,
        author_email: &str,
        author_ts: Timestamp,
        committer_ts: Timestamp,
    ) -> String {
        fs::write(self.path.join(file_name), contents).expect("write file in temp repo");
        self.git(&["add", file_name]);
        let author_date = format!("@{}", author_ts.as_unix_secs());
        let committer_date = format!("@{}", committer_ts.as_unix_secs());
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.path)
            .args(["-c", "commit.gpgsign=false", "commit", "-q", "-m", message])
            .env("HOME", &self.home)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_AUTHOR_NAME", author_name)
            .env("GIT_AUTHOR_EMAIL", author_email)
            .env("GIT_AUTHOR_DATE", &author_date)
            .env("GIT_COMMITTER_NAME", author_name)
            .env("GIT_COMMITTER_EMAIL", author_email)
            .env("GIT_COMMITTER_DATE", &committer_date)
            .output()
            .expect("run git commit");
        assert!(
            output.status.success(),
            "git commit failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        self.git_stdout(&["rev-parse", "HEAD"]).trim().to_string()
    }

    fn amend_head(&self, new_message: &str, author_email: &str, committer_ts: Timestamp) -> String {
        let committer_date = format!("@{}", committer_ts.as_unix_secs());
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.path)
            .args([
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--amend",
                "-q",
                "-m",
                new_message,
            ])
            .env("HOME", &self.home)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_COMMITTER_NAME", "Amend Committer")
            .env("GIT_COMMITTER_EMAIL", author_email)
            .env("GIT_COMMITTER_DATE", &committer_date)
            .output()
            .expect("run git commit --amend");
        assert!(
            output.status.success(),
            "git commit --amend failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        self.git_stdout(&["rev-parse", "HEAD"]).trim().to_string()
    }

    fn rebase_onto(&self, upstream: &str, author_email: &str, committer_ts: Timestamp) {
        let committer_date = format!("@{}", committer_ts.as_unix_secs());
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.path)
            .args(["-c", "commit.gpgsign=false", "rebase", "-q", upstream])
            .env("HOME", &self.home)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_COMMITTER_NAME", "Rebase Committer")
            .env("GIT_COMMITTER_EMAIL", author_email)
            .env("GIT_COMMITTER_DATE", &committer_date)
            .output()
            .expect("run git rebase");
        assert!(
            output.status.success(),
            "git rebase failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn git_log_parser_and_tilde_expansion_handle_valid_malformed_and_home_paths() {
    let raw = "\
0123456789abcdef0123456789abcdef01234567 1700000010 1700000020

not-enough-fields 1700000010
fedcba9876543210fedcba9876543210fedcba98 bad_ts 1700000030
abcdefabcdefabcdefabcdefabcdefabcdefabcd 1700000040 1700000050
";
    let parsed = parse_git_log_output(raw);
    assert_eq!(
        parsed,
        vec![
            ObservedGitCommit {
                oid: "0123456789abcdef0123456789abcdef01234567".to_string(),
                author_ts: Timestamp::from_unix_secs(1_700_000_010),
                committer_ts: Timestamp::from_unix_secs(1_700_000_020),
            },
            ObservedGitCommit {
                oid: "abcdefabcdefabcdefabcdefabcdefabcdefabcd".to_string(),
                author_ts: Timestamp::from_unix_secs(1_700_000_040),
                committer_ts: Timestamp::from_unix_secs(1_700_000_050),
            },
        ]
    );

    let fake_home = Path::new("/tmp/niwa-fake-home");
    assert_eq!(
        expand_repo_path(Path::new("~/repos/niwa"), Some(fake_home)),
        PathBuf::from("/tmp/niwa-fake-home/repos/niwa")
    );
    assert_eq!(
        expand_repo_path(Path::new("~"), Some(fake_home)),
        PathBuf::from("/tmp/niwa-fake-home")
    );
    assert_eq!(
        expand_repo_path(Path::new("~otheruser/repos/niwa"), Some(fake_home)),
        PathBuf::from("~otheruser/repos/niwa")
    );
    assert_eq!(
        expand_repo_path(Path::new("/var/tmp/repo"), Some(fake_home)),
        PathBuf::from("/var/tmp/repo")
    );
}

#[test]
fn poll_gates_on_consent_and_config_and_counts_only_configured_authors_new_commits() {
    let env = TestEnv::new();
    let spool = EventSpool::new(env.paths().clone());
    let t_consent = Timestamp::from_unix_secs(1_700_000_000);
    let now = t_consent.add_secs(300);

    let repo = TempGitRepo::init(&env, "work_repo");
    let empty_unborn_repo = TempGitRepo::init(&env, "empty_unborn_repo");
    let missing_repo_path = env.home_dir().join("repos").join("does_not_exist");

    // 1. Pre-consent commit by Alice (`t_consent - 120`): MUST be ignored.
    let _ = repo.commit_at(
        "pre.txt",
        "pre-consent",
        "pre-consent commit",
        "Alice",
        "alice@example.test",
        t_consent.add_secs(-120),
        t_consent.add_secs(-120),
    );
    // 2. Post-consent commit by Bob: MUST be ignored.
    let _ = repo.commit_at(
        "bob.txt",
        "bob work",
        "bob commit",
        "Bob",
        "bob@example.test",
        t_consent.add_secs(30),
        t_consent.add_secs(30),
    );
    // 3. Post-consent commit by substring lookalike `not-alice@example.test`: MUST be ignored.
    let _ = repo.commit_at(
        "lookalike.txt",
        "not alice",
        "lookalike author email",
        "Not Alice",
        "not-alice@example.test",
        t_consent.add_secs(45),
        t_consent.add_secs(45),
    );
    // 4. Two post-consent commits by Alice in the exact same Unix second (`t_consent + 90`)
    //    plus one uppercase-email commit (`ALICE@EXAMPLE.TEST`) at `t_consent + 180`: MUST count (3).
    let _ = repo.commit_at(
        "alice1.txt",
        "alice 1",
        "alice commit 1",
        "Alice",
        "alice@example.test",
        t_consent.add_secs(90),
        t_consent.add_secs(90),
    );
    let _ = repo.commit_at(
        "alice2.txt",
        "alice 2",
        "alice commit 2 same second",
        "Alice",
        "alice@example.test",
        t_consent.add_secs(90),
        t_consent.add_secs(90),
    );
    let _ = repo.commit_at(
        "alice3.txt",
        "alice 3",
        "alice uppercase email",
        "Alice",
        "ALICE@EXAMPLE.TEST",
        t_consent.add_secs(180),
        t_consent.add_secs(180),
    );
    // 5. Future commit relative to `now` (`now + 120`): deferred until clock reaches `now + 120`.
    let _ = repo.commit_at(
        "alice_future.txt",
        "alice future",
        "alice future commit",
        "Alice",
        "alice@example.test",
        now.add_secs(120),
        now.add_secs(120),
    );

    let mut config = AppConfig::default();
    let mut poll_state = GitPollState::default();
    let fake_runner = FakeGitLogRunner::default();

    // Disabled signal: zero runner invocations and no spool file created.
    let disabled = poll_and_record_git_commits(
        &config,
        &spool,
        &mut poll_state,
        None,
        now,
        GitPollMode::OnCatchUp,
        &fake_runner,
    )
    .unwrap();
    assert_eq!(disabled, GitPollOutcome::Disabled);
    assert_eq!(fake_runner.calls(), 0);
    assert!(!env.paths().events_file().exists());

    // Enabled signal, but `git.repos` / `git.author_email` not yet configured.
    config.enable_signal(SignalKind::Git, t_consent);
    let unconfigured = poll_and_record_git_commits(
        &config,
        &spool,
        &mut poll_state,
        None,
        now,
        GitPollMode::OnCatchUp,
        &fake_runner,
    )
    .unwrap();
    assert_eq!(unconfigured, GitPollOutcome::Unconfigured);
    assert_eq!(fake_runner.calls(), 0);

    // Configure `~/repos/work_repo` (testing tilde expansion against `env.home_dir()`),
    // the unborn 0-commit repo, and the nonexistent path.
    config.git.repos = vec![
        PathBuf::from("~/repos/work_repo"),
        empty_unborn_repo.path().to_path_buf(),
        missing_repo_path,
    ];
    config.git.author_email = Some("alice@example.test".to_string());

    let runner = CountingGitLogRunner::new(ProcessGitLogRunner::with_home(env.home_dir()));
    let outcome = poll_and_record_git_commits(
        &config,
        &spool,
        &mut poll_state,
        None,
        now,
        GitPollMode::OnCatchUp,
        &runner,
    )
    .unwrap();
    assert_eq!(
        outcome,
        GitPollOutcome::Polled {
            repos_queried: 3,
            repos_succeeded: 2,
            observed_commits: 3,
            events_recorded: 3,
        }
    );
    assert_eq!(poll_state.consent_anchor, Some(t_consent));
    assert_eq!(poll_state.last_poll_at, Some(now));
    assert_eq!(poll_state.credited_commits, 3);

    let events = spool.read().unwrap().events;
    assert_eq!(
        events,
        vec![
            WorkEvent::GitCommit {
                ts: t_consent.add_secs(90)
            },
            WorkEvent::GitCommit {
                ts: t_consent.add_secs(90)
            },
            WorkEvent::GitCommit {
                ts: t_consent.add_secs(180)
            },
        ]
    );

    // Advance `now` past the 4th commit (`now + 120`): records ONLY that 1 newly reached commit.
    let later = now.add_secs(180);
    let outcome_later = poll_and_record_git_commits(
        &config,
        &spool,
        &mut poll_state,
        None,
        later,
        GitPollMode::OnCatchUp,
        &runner,
    )
    .unwrap();
    assert_eq!(
        outcome_later,
        GitPollOutcome::Polled {
            repos_queried: 3,
            repos_succeeded: 2,
            observed_commits: 4,
            events_recorded: 1,
        }
    );
    assert_eq!(poll_state.credited_commits, 4);
    assert_eq!(spool.read().unwrap().events.len(), 4);

    // Confirm read-only polling left the repository clean with no `.git/index.lock`.
    assert!(repo
        .git_stdout(&["status", "--porcelain"])
        .trim()
        .is_empty());
    assert!(!repo.path().join(".git").join("index.lock").exists());
}

#[test]
fn poll_dedupes_across_clones_and_re_polls_and_throttles_periodic_5_min_interval() {
    assert_eq!(GIT_POLL_INTERVAL_SECS, 300);

    let env = TestEnv::new();
    let spool = EventSpool::new(env.paths().clone());
    let t0 = Timestamp::from_unix_secs(1_700_010_000);

    let repo_a = TempGitRepo::init(&env, "repo_a");
    let _ = repo_a.commit_at(
        "a.txt",
        "shared commit",
        "shared commit in repo_a",
        "Alice",
        "alice@example.test",
        t0.add_secs(60),
        t0.add_secs(60),
    );
    // Clone `repo_a` so `repo_a_clone` contains the exact same commit OID.
    let repo_a_clone = TempGitRepo::clone_from(&env, repo_a.path(), "repo_a_clone");

    // Independent `repo_b` with a distinct commit at the EXACT same timestamp (`t0 + 60`).
    let repo_b = TempGitRepo::init(&env, "repo_b");
    let _ = repo_b.commit_at(
        "b.txt",
        "independent commit",
        "distinct commit in repo_b at identical timestamp",
        "Alice",
        "alice@example.test",
        t0.add_secs(60),
        t0.add_secs(60),
    );

    let mut config = AppConfig::default();
    config.enable_signal(SignalKind::Git, t0);
    config.git.author_email = Some("alice@example.test".to_string());
    config.git.repos = vec![
        repo_a.path().to_path_buf(),
        repo_a_clone.path().to_path_buf(),
        repo_b.path().to_path_buf(),
    ];

    let mut poll_state = GitPollState::default();
    let runner = CountingGitLogRunner::new(ProcessGitLogRunner::with_home(env.home_dir()));

    // First poll at `t0 + 120`: queries all 3 repos, dedupes `repo_a` + `repo_a_clone` (1)
    // and keeps `repo_b` (1) -> 2 unique commits recorded.
    let first = poll_and_record_git_commits(
        &config,
        &spool,
        &mut poll_state,
        None,
        t0.add_secs(120),
        GitPollMode::PeriodicInterval,
        &runner,
    )
    .unwrap();
    assert_eq!(
        first,
        GitPollOutcome::Polled {
            repos_queried: 3,
            repos_succeeded: 3,
            observed_commits: 2,
            events_recorded: 2,
        }
    );
    assert_eq!(runner.calls(), 3);
    assert_eq!(spool.read().unwrap().events.len(), 2);

    // Periodic polls before `+300 s` (`+60 s` and `+299 s` after `t0 + 120`) are throttled
    // without spawning a single `git` subprocess.
    for delta in [60_i64, 180, 299] {
        let throttled = poll_and_record_git_commits(
            &config,
            &spool,
            &mut poll_state,
            None,
            t0.add_secs(120 + delta),
            GitPollMode::PeriodicInterval,
            &runner,
        )
        .unwrap();
        assert_eq!(throttled, GitPollOutcome::Throttled);
        assert_eq!(runner.calls(), 3);
    }

    // Explicit catch-up (`GitPollMode::OnCatchUp`) bypasses the periodic throttle immediately,
    // and re-polling with no new commits records 0 duplicates.
    let catchup_repoll = poll_and_record_git_commits(
        &config,
        &spool,
        &mut poll_state,
        None,
        t0.add_secs(200),
        GitPollMode::OnCatchUp,
        &runner,
    )
    .unwrap();
    assert_eq!(
        catchup_repoll,
        GitPollOutcome::Polled {
            repos_queried: 3,
            repos_succeeded: 3,
            observed_commits: 2,
            events_recorded: 0,
        }
    );
    assert_eq!(runner.calls(), 6);
    assert_eq!(spool.read().unwrap().events.len(), 2);

    // At `last_poll_at + 300 s` (`t0 + 500`), `PeriodicInterval` polls again cleanly.
    let periodic_after_5m = poll_and_record_git_commits(
        &config,
        &spool,
        &mut poll_state,
        None,
        t0.add_secs(500),
        GitPollMode::PeriodicInterval,
        &runner,
    )
    .unwrap();
    assert_eq!(
        periodic_after_5m,
        GitPollOutcome::Polled {
            repos_queried: 3,
            repos_succeeded: 3,
            observed_commits: 2,
            events_recorded: 0,
        }
    );
    assert_eq!(runner.calls(), 9);
    assert_eq!(spool.read().unwrap().events.len(), 2);
}

#[test]
fn rewritten_history_amend_and_rebase_never_double_count() {
    let env = TestEnv::new();
    let spool = EventSpool::new(env.paths().clone());
    let t_consent = Timestamp::from_unix_secs(1_700_020_000);

    let repo = TempGitRepo::init(&env, "rewrite_repo");
    // Base commit before consent on `main`.
    let _ = repo.commit_at(
        "base.txt",
        "base",
        "initial base before consent",
        "Alice",
        "alice@example.test",
        t_consent.add_secs(-600),
        t_consent.add_secs(-600),
    );

    // Legacy pre-consent branch created before `t_consent` that will later be rebased after `t_consent`.
    repo.git(&["checkout", "-q", "-b", "legacy-pre-consent"]);
    let _ = repo.commit_at(
        "legacy.txt",
        "legacy pre-consent work",
        "legacy commit authored before consent",
        "Alice",
        "alice@example.test",
        t_consent.add_secs(-300),
        t_consent.add_secs(-300),
    );

    // Feature branch with 2 post-consent commits by Alice.
    repo.git(&["checkout", "-q", "main"]);
    repo.git(&["checkout", "-q", "-b", "feature"]);
    let _ = repo.commit_at(
        "feat1.txt",
        "v1",
        "feature commit 1",
        "Alice",
        "alice@example.test",
        t_consent.add_secs(30),
        t_consent.add_secs(30),
    );
    let orig_head = repo.commit_at(
        "feat2.txt",
        "v2",
        "feature commit 2",
        "Alice",
        "alice@example.test",
        t_consent.add_secs(60),
        t_consent.add_secs(60),
    );

    let mut config = AppConfig::default();
    config.enable_signal(SignalKind::Git, t_consent);
    config.git.author_email = Some("alice@example.test".to_string());
    config.git.repos = vec![repo.path().to_path_buf()];

    let mut poll_state = GitPollState::default();
    let runner = ProcessGitLogRunner::with_home(env.home_dir());

    // 1. Initial poll credits Alice's 2 post-consent commits.
    let initial = poll_and_record_git_commits(
        &config,
        &spool,
        &mut poll_state,
        None,
        t_consent.add_secs(90),
        GitPollMode::OnCatchUp,
        &runner,
    )
    .unwrap();
    assert_eq!(
        initial,
        GitPollOutcome::Polled {
            repos_queried: 1,
            repos_succeeded: 1,
            observed_commits: 2,
            events_recorded: 2,
        }
    );
    assert_eq!(spool.read().unwrap().events.len(), 2);

    // 2. `git commit --amend` produces a brand-new OID and newer committer timestamp (`t_consent + 120`):
    //    must NOT double count (`events_recorded == 0`).
    let amended_head = repo.amend_head(
        "feature commit 2 (amended)",
        "alice@example.test",
        t_consent.add_secs(120),
    );
    assert_ne!(orig_head, amended_head);

    let after_amend = poll_and_record_git_commits(
        &config,
        &spool,
        &mut poll_state,
        None,
        t_consent.add_secs(150),
        GitPollMode::OnCatchUp,
        &runner,
    )
    .unwrap();
    assert_eq!(
        after_amend,
        GitPollOutcome::Polled {
            repos_queried: 1,
            repos_succeeded: 1,
            observed_commits: 2,
            events_recorded: 0,
        }
    );
    assert_eq!(spool.read().unwrap().events.len(), 2);

    // 3. Bob adds a new upstream commit on `main`, and Alice rebases both `legacy-pre-consent`
    //    (authored before `t_consent`, rebased at `t_consent + 200`) AND `feature` (2 post-consent
    //    commits, rebased at `t_consent + 220`) onto `main`. Neither rebase may double-count!
    repo.git(&["checkout", "-q", "main"]);
    let _ = repo.commit_at(
        "upstream_bob.txt",
        "bob upstream",
        "upstream commit by bob",
        "Bob",
        "bob@example.test",
        t_consent.add_secs(180),
        t_consent.add_secs(180),
    );

    repo.git(&["checkout", "-q", "legacy-pre-consent"]);
    repo.rebase_onto("main", "alice@example.test", t_consent.add_secs(200));

    repo.git(&["checkout", "-q", "feature"]);
    repo.rebase_onto("main", "alice@example.test", t_consent.add_secs(220));
    let rebased_head = repo.git_stdout(&["rev-parse", "HEAD"]).trim().to_string();
    assert_ne!(amended_head, rebased_head);

    let after_rebase = poll_and_record_git_commits(
        &config,
        &spool,
        &mut poll_state,
        None,
        t_consent.add_secs(240),
        GitPollMode::OnCatchUp,
        &runner,
    )
    .unwrap();
    assert_eq!(
        after_rebase,
        GitPollOutcome::Polled {
            repos_queried: 1,
            repos_succeeded: 1,
            observed_commits: 2,
            events_recorded: 0,
        }
    );
    assert_eq!(spool.read().unwrap().events.len(), 2);

    // 4. Adding 1 genuinely new commit after the rebase credits ONLY that 1 new commit (`t_consent + 270`).
    let _ = repo.commit_at(
        "feat3.txt",
        "v3",
        "feature commit 3 after rebase",
        "Alice",
        "alice@example.test",
        t_consent.add_secs(270),
        t_consent.add_secs(270),
    );
    let after_new_post_rebase = poll_and_record_git_commits(
        &config,
        &spool,
        &mut poll_state,
        None,
        t_consent.add_secs(300),
        GitPollMode::OnCatchUp,
        &runner,
    )
    .unwrap();
    assert_eq!(
        after_new_post_rebase,
        GitPollOutcome::Polled {
            repos_queried: 1,
            repos_succeeded: 1,
            observed_commits: 3,
            events_recorded: 1,
        }
    );
    let final_events = spool.read().unwrap().events;
    assert_eq!(final_events.len(), 3);
    assert_eq!(
        final_events[2],
        WorkEvent::GitCommit {
            ts: t_consent.add_secs(270)
        }
    );
}
