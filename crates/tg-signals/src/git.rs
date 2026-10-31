//! Opt-in read-only `git log` signal poller, cross-repo / rewritten-history deduplicator,
//! and 5-minute periodic throttle (`PLAN.md` §0.4, §0.6, §0.7, §5, §7 & §10 T11).

use crate::consent::{is_signal_enabled, record_if_consented};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use tg_core::clock::Timestamp;
use tg_store::config::{AppConfig, SignalKind};
use tg_store::error::StoreError;
use tg_store::save::GitPollState;
use tg_store::spool::{EventSpool, WorkEvent};

/// Minimum interval (`300 s` = `5 min`) between periodic background `git log` polls
/// (`PLAN.md` §0.7, §5 & §7).
///
/// Power-budget justification (`PLAN.md` §0.7 & §7):
/// - `niwa daemon` wakes on a coalesced `60 s` tick (`TICK_SECS`) and the interactive TUI
///   advances ticks every `60 s`. Throttling periodic background git polls to `300 s` (`5 min`)
///   bounds read-only `git log` subprocesses to at most once every 5 ticks with zero filesystem
///   watchers, while explicit CLI/TUI catch-up ([`GitPollMode::OnCatchUp`]) refreshes immediately.
pub const GIT_POLL_INTERVAL_SECS: i64 = 5 * 60;

/// Controls whether [`poll_and_record_git_commits`] runs immediately or respects the
/// 5-minute periodic throttle ([`GIT_POLL_INTERVAL_SECS`]).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum GitPollMode {
    /// Immediate poll on explicit catch-up (`niwa`, `niwa peek`, `niwa status`).
    OnCatchUp,
    /// Periodic background poll (`niwa daemon` and open-TUI tick advancement), throttled to at
    /// most once every [`GIT_POLL_INTERVAL_SECS`] (`300 s`).
    PeriodicInterval,
}

/// Ephemeral in-memory commit metadata parsed from `git log --format=%H %at %ct`.
///
/// Privacy invariant (`PLAN.md` §0.4 & §10 T11):
/// - Intentionally does **not** implement `Serialize` or `Deserialize`.
/// - `oid` exists strictly in transient memory to deduplicate shared commits across clones or
///   worktrees of the same repository and never touches `events.jsonl` or `garden.json`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservedGitCommit {
    pub oid: String,
    pub author_ts: Timestamp,
    pub committer_ts: Timestamp,
}

/// Read-only adapter querying local commits by `author_email` in `repo_path`.
pub trait GitLogRunner {
    /// Queries local branch and `HEAD` commits in `repo_path` matching `author_email` since `since`,
    /// returning `None` if `repo_path` is missing or not a readable git repository.
    fn query_repo_commits(
        &self,
        repo_path: &Path,
        author_email: &str,
        since: Timestamp,
    ) -> Option<Vec<ObservedGitCommit>>;
}

/// Production read-only [`GitLogRunner`] invoking `git --no-optional-locks log`.
#[derive(Clone, Debug, Default)]
pub struct ProcessGitLogRunner {
    home_override: Option<PathBuf>,
}

impl ProcessGitLogRunner {
    /// Creates a [`ProcessGitLogRunner`] resolving leading `~/` repo paths against `home_dir`.
    #[must_use]
    pub fn with_home(home_dir: impl Into<PathBuf>) -> Self {
        Self {
            home_override: Some(home_dir.into()),
        }
    }

    fn resolve_home(&self) -> Option<PathBuf> {
        self.home_override
            .clone()
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
    }
}

impl GitLogRunner for ProcessGitLogRunner {
    fn query_repo_commits(
        &self,
        repo_path: &Path,
        author_email: &str,
        since: Timestamp,
    ) -> Option<Vec<ObservedGitCommit>> {
        let trimmed_email = author_email.trim();
        let normalized_email = trimmed_email
            .strip_prefix('<')
            .and_then(|s| s.strip_suffix('>'))
            .map_or(trimmed_email, str::trim);
        if normalized_email.is_empty() {
            return None;
        }

        let home = self.resolve_home();
        let expanded = expand_repo_path(repo_path, home.as_deref());
        if !expanded.exists() {
            return None;
        }

        // Subtract 1 second as a coarse bound for git's C revision walker so commits at the
        // exact `since` second are never skipped by git version differences; exact `>= anchor`
        // filtering is enforced deterministically in Rust.
        let coarse_since = since.as_unix_secs().saturating_sub(1).max(0);
        let since_arg = format!("--since=@{coarse_since}");
        let author_arg = format!("--author=<{normalized_email}>");

        // Query `--branches HEAD` (local branches + `HEAD`, excluding `refs/remotes/*` and reflogs
        // so amended/rebased commits drop out immediately). Fall back to `--branches` if `HEAD` is
        // unborn (e.g. a newly `git init`'d repository with 0 commits).
        let stdout = run_readonly_git_log(&expanded, &since_arg, &author_arg, true)
            .or_else(|| run_readonly_git_log(&expanded, &since_arg, &author_arg, false))?;
        Some(parse_git_log_output(&stdout))
    }
}

/// Decorator wrapping any [`GitLogRunner`] with an atomic invocation counter for power-budget tests.
#[derive(Debug)]
pub struct CountingGitLogRunner<R> {
    inner: R,
    calls: AtomicUsize,
}

impl<R> CountingGitLogRunner<R> {
    #[must_use]
    pub const fn new(inner: R) -> Self {
        Self {
            inner,
            calls: AtomicUsize::new(0),
        }
    }

    #[must_use]
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl<R: GitLogRunner> GitLogRunner for CountingGitLogRunner<R> {
    fn query_repo_commits(
        &self,
        repo_path: &Path,
        author_email: &str,
        since: Timestamp,
    ) -> Option<Vec<ObservedGitCommit>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner
            .query_repo_commits(repo_path, author_email, since)
    }
}

/// Pure in-memory test double implementing [`GitLogRunner`].
#[derive(Debug, Default)]
pub struct FakeGitLogRunner {
    repos: Mutex<BTreeMap<PathBuf, Option<Vec<ObservedGitCommit>>>>,
    calls: AtomicUsize,
}

impl FakeGitLogRunner {
    pub fn set_repo_commits(
        &self,
        repo_path: impl Into<PathBuf>,
        commits: Option<Vec<ObservedGitCommit>>,
    ) {
        let mut guard = self.repos.lock().expect("FakeGitLogRunner mutex poisoned");
        guard.insert(repo_path.into(), commits);
    }

    #[must_use]
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl GitLogRunner for FakeGitLogRunner {
    fn query_repo_commits(
        &self,
        repo_path: &Path,
        _author_email: &str,
        _since: Timestamp,
    ) -> Option<Vec<ObservedGitCommit>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let guard = self.repos.lock().expect("FakeGitLogRunner mutex poisoned");
        guard.get(repo_path).cloned().flatten()
    }
}

/// Expands a leading `~` or `~/...` prefix against `home_dir` while leaving `~otheruser/...`
/// and non-tilde paths untouched.
#[must_use]
pub fn expand_repo_path(repo_path: &Path, home_dir: Option<&Path>) -> PathBuf {
    let Some(home) = home_dir else {
        return repo_path.to_path_buf();
    };
    if repo_path == Path::new("~") {
        return home.to_path_buf();
    }
    if let Ok(stripped) = repo_path.strip_prefix("~") {
        return home.join(stripped);
    }
    repo_path.to_path_buf()
}

/// Parses `git log --format=%H %at %ct` stdout lines into ephemeral [`ObservedGitCommit`]s.
///
/// Malformed or blank lines are skipped safely without panicking.
#[must_use]
pub fn parse_git_log_output(stdout: &str) -> Vec<ObservedGitCommit> {
    let mut commits = Vec::new();
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        let (Some(oid), Some(author_raw), Some(committer_raw), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        if oid.is_empty() || !oid.bytes().all(|b| b.is_ascii_hexdigit()) {
            continue;
        }
        let (Ok(author_secs), Ok(committer_secs)) =
            (author_raw.parse::<i64>(), committer_raw.parse::<i64>())
        else {
            continue;
        };
        commits.push(ObservedGitCommit {
            oid: oid.to_ascii_lowercase(),
            author_ts: Timestamp::from_unix_secs(author_secs),
            committer_ts: Timestamp::from_unix_secs(committer_secs),
        });
    }
    commits
}

/// Outcome returned by [`poll_and_record_git_commits`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum GitPollOutcome {
    /// `signals.git` is disabled or lacks a valid consent window (`now < anchor`);
    /// no `git` subprocess was spawned.
    Disabled,
    /// `git.repos` is empty or `git.author_email` is unset; no `git` subprocess was spawned.
    Unconfigured,
    /// [`GitPollMode::PeriodicInterval`] skipped polling because fewer than
    /// [`GIT_POLL_INTERVAL_SECS`] (`300 s`) elapsed since `poll_state.last_poll_at`;
    /// no `git` subprocess was spawned.
    Throttled,
    /// Polled configured repositories read-only and appended `events_recorded` new
    /// [`WorkEvent::GitCommit`] timestamps to `events.jsonl`.
    Polled {
        repos_queried: usize,
        repos_succeeded: usize,
        observed_commits: u64,
        events_recorded: usize,
    },
}

/// Polls configured `git.repos` read-only for `git.author_email`, deduplicates across repositories,
/// re-polls, and rewritten history (`git commit --amend` / `git rebase`), and appends new commit
/// timestamps only via [`record_if_consented`] (`PLAN.md` §0.4, §5 & §10 T11).
pub fn poll_and_record_git_commits(
    config: &AppConfig,
    spool: &EventSpool,
    poll_state: &mut GitPollState,
    floor_ts: Option<Timestamp>,
    now: Timestamp,
    mode: GitPollMode,
    runner: &dyn GitLogRunner,
) -> Result<GitPollOutcome, StoreError> {
    if !is_signal_enabled(config, SignalKind::Git) {
        return Ok(GitPollOutcome::Disabled);
    }
    let Some(consented_at) = config.signals.git.consented_at else {
        return Ok(GitPollOutcome::Disabled);
    };

    let effective_anchor = floor_ts.map_or(consented_at, |floor| consented_at.max(floor));
    if now < effective_anchor {
        return Ok(GitPollOutcome::Disabled);
    }

    let Some(author_email) = config
        .git
        .author_email
        .as_deref()
        .map(str::trim)
        .filter(|email| !email.is_empty())
    else {
        return Ok(GitPollOutcome::Unconfigured);
    };
    if config.git.repos.is_empty() {
        return Ok(GitPollOutcome::Unconfigured);
    }

    if poll_state.consent_anchor != Some(effective_anchor) {
        poll_state.consent_anchor = Some(effective_anchor);
        poll_state.credited_commits = 0;
        poll_state.last_poll_at = None;
    }

    if mode == GitPollMode::PeriodicInterval {
        if let Some(last_poll) = poll_state.last_poll_at {
            let elapsed = now.as_unix_secs().saturating_sub(last_poll.as_unix_secs());
            if now >= last_poll && elapsed < GIT_POLL_INTERVAL_SECS {
                return Ok(GitPollOutcome::Throttled);
            }
        }
    }

    let repos_queried = config.git.repos.len();
    let mut repos_succeeded = 0_usize;
    let mut unique_by_oid: HashMap<String, ObservedGitCommit> = HashMap::new();

    for repo_path in &config.git.repos {
        if let Some(repo_commits) =
            runner.query_repo_commits(repo_path, author_email, effective_anchor)
        {
            repos_succeeded = repos_succeeded.saturating_add(1);
            for commit in repo_commits {
                // Require BOTH author_ts and committer_ts >= effective_anchor so pre-consent
                // commits that are later rebased after consent (`committer_ts >= anchor` while
                // `author_ts < anchor`) are never counted, and defer future `committer_ts > now`.
                if commit.author_ts >= effective_anchor
                    && commit.committer_ts >= effective_anchor
                    && commit.committer_ts <= now
                {
                    unique_by_oid.entry(commit.oid.clone()).or_insert(commit);
                }
            }
        }
    }

    let mut ordered: Vec<ObservedGitCommit> = unique_by_oid.into_values().collect();
    ordered.sort_by(|a, b| {
        (a.committer_ts, a.author_ts, &a.oid).cmp(&(b.committer_ts, b.author_ts, &b.oid))
    });

    poll_state.last_poll_at = Some(now);
    let observed_commits = u64::try_from(ordered.len()).unwrap_or(u64::MAX);

    if observed_commits <= poll_state.credited_commits {
        return Ok(GitPollOutcome::Polled {
            repos_queried,
            repos_succeeded,
            observed_commits,
            events_recorded: 0,
        });
    }

    let start_idx = usize::try_from(poll_state.credited_commits).unwrap_or(ordered.len());
    let mut events_recorded = 0_usize;
    for commit in &ordered[start_idx..] {
        let ev = WorkEvent::GitCommit {
            ts: commit.committer_ts,
        };
        if record_if_consented(config, spool, ev)? {
            events_recorded = events_recorded.saturating_add(1);
        }
    }
    poll_state.credited_commits = observed_commits;

    Ok(GitPollOutcome::Polled {
        repos_queried,
        repos_succeeded,
        observed_commits,
        events_recorded,
    })
}

fn run_readonly_git_log(
    repo_dir: &Path,
    since_arg: &str,
    author_arg: &str,
    include_head: bool,
) -> Option<String> {
    let mut cmd = Command::new("git");
    cmd.args([
        "--no-optional-locks",
        "-c",
        "gc.auto=0",
        "-c",
        "maintenance.auto=false",
        "-C",
    ]);
    cmd.arg(repo_dir);
    cmd.args([
        "log",
        since_arg,
        "--fixed-strings",
        "--regexp-ignore-case",
        author_arg,
        "--format=%H %at %ct",
        "--branches",
    ]);
    if include_head {
        cmd.arg("HEAD");
    }

    let output = cmd
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}
