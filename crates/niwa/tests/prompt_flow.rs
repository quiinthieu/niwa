#[path = "../../../tests/support/mod.rs"]
mod support;

use assert_cmd::Command;
use niwa_garden::prompt::{run_prompt_with_writer, PromptArgs};
use niwa_garden::runtime::GardenSession;
use std::fs;
use std::path::PathBuf;
use support::TestEnv;
use tg_core::clock::{Clock, FakeClock, Timestamp};
use tg_core::season::Season;
use tg_store::clock::SystemClock;
use tg_store::config::{ConfigStore, PromptStyle, SignalKind};
use tg_store::lock::GardenLock;
use tg_store::snapshot::{read_snapshot, write_snapshot, Snapshot};
use tg_store::spool::{DurationBucket, EventSpool, ShellExitClass, WorkEvent};

fn niwa_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_niwa"))
}

fn niwa_cmd(env: &TestEnv) -> Command {
    let mut cmd = Command::new(niwa_bin());
    cmd.envs(env.env_vars());
    cmd
}

fn run_prompt_capture(env: &TestEnv, clock: &dyn Clock, args: PromptArgs) -> String {
    let mut out = Vec::new();
    run_prompt_with_writer(env.paths(), clock, &args, &mut out).expect("run_prompt_with_writer");
    String::from_utf8(out).expect("prompt output is UTF-8")
}

#[test]
fn prompt_fast_path_never_runs_simulation_or_acquires_lock_when_snapshot_is_fresh() {
    let env = TestEnv::new();
    let t0 = Timestamp::from_unix_secs(1_745_000_000);
    let clock = FakeClock::new(t0);

    // Enable shell signal and queue an unapplied spool event so we can prove `prompt` never drains it.
    let cfg_store = ConfigStore::new(env.paths().clone());
    let mut cfg = cfg_store.load().unwrap();
    cfg.enable_signal(SignalKind::Shell, t0);
    cfg_store.save(&cfg).unwrap();

    // Seed a fresh snapshot (`updated_at = t0`).
    let mut snap = Snapshot::new(
        "◈",
        "2/8 planted · 1 blooming",
        Season::Spring,
        "Niwa · Spring",
        t0,
    );
    snap.planted_count = 2;
    snap.blooming_count = 1;
    write_snapshot(env.paths(), &snap).unwrap();
    let original_snapshot_bytes = fs::read(env.paths().snapshot_file()).unwrap();

    // Corrupt `garden.json` on disk: if `prompt` ever attempted to load `garden.json`, it would fail!
    let corrupt_garden_bytes = b"CORRUPT_IF_PROMPT_RUNS_SIMULATION";
    fs::write(env.paths().garden_file(), corrupt_garden_bytes).unwrap();

    // Append a pending spool event: if `prompt` ever ran catch-up, it would compact `events.jsonl`!
    let spool = EventSpool::new(env.paths().clone());
    spool
        .append(WorkEvent::Shell {
            ts: t0,
            status: ShellExitClass::Ok,
            dur_bucket: DurationBucket::Fast,
        })
        .unwrap();

    // Hold an exclusive advisory lock on `garden.lock` in this thread: if `prompt` tried to acquire
    // `garden.lock`, it would deadlock/block!
    let _held_lock = GardenLock::acquire(env.paths()).expect("hold exclusive garden.lock");

    // 1. Fresh at `age == 0 s` and `age == 59 s` (`< 60 s`) via `FakeClock`.
    assert_eq!(
        run_prompt_capture(&env, &clock, PromptArgs::default()),
        "◈\n"
    );
    clock.advance_secs(59);
    assert_eq!(
        run_prompt_capture(&env, &clock, PromptArgs::default()),
        "◈\n"
    );
    assert_eq!(
        run_prompt_capture(
            &env,
            &clock,
            PromptArgs {
                style: Some(PromptStyle::Emoji),
                ..PromptArgs::default()
            }
        ),
        "🌸\n"
    );

    // 2. Also exercise the real `niwa prompt` subprocess (`main.rs` direct fast path) while
    // `garden.lock` is STILL held and `garden.json` is STILL corrupt!
    let live_now = SystemClock.now();
    snap.updated_at = live_now;
    write_snapshot(env.paths(), &snap).unwrap();
    let live_snapshot_bytes = fs::read(env.paths().snapshot_file()).unwrap();

    let sub_glyph = niwa_cmd(&env)
        .arg("prompt")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(String::from_utf8(sub_glyph).unwrap(), "◈\n");

    let sub_emoji = niwa_cmd(&env)
        .args(["prompt", "--style", "emoji"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(String::from_utf8(sub_emoji).unwrap(), "🌸\n");

    let sub_none = niwa_cmd(&env)
        .args(["prompt", "--style=none"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert!(sub_none.is_empty(), "`--style none` must emit 0 bytes");

    // Verify neither `garden.json`, `snapshot.json`, nor `events.jsonl` was touched!
    assert_eq!(
        fs::read(env.paths().garden_file()).unwrap(),
        corrupt_garden_bytes
    );
    assert_eq!(
        fs::read(env.paths().snapshot_file()).unwrap(),
        live_snapshot_bytes
    );
    assert_eq!(spool.read().unwrap().events.len(), 1);
    assert_ne!(original_snapshot_bytes.len(), 0);
}

#[test]
fn prompt_refreshes_stale_or_missing_snapshot_and_never_blocks_when_lock_is_contended() {
    let env = TestEnv::new();
    let t0 = Timestamp::from_unix_secs(1_745_000_000);
    let clock = FakeClock::new(t0);

    // 1. Missing `snapshot.json`: `prompt` initializes the starter garden and writes `snapshot.json`.
    assert!(!env.paths().snapshot_file().exists());
    assert_eq!(
        run_prompt_capture(&env, &clock, PromptArgs::default()),
        "◌\n"
    );
    let initial_snap = read_snapshot(env.paths())
        .unwrap()
        .expect("snapshot created");
    assert_eq!(initial_snap.updated_at, t0);
    assert_eq!(initial_snap.glyph, "◌");

    // Plant a starter seed so growth across ticks changes the compact glyph.
    {
        let mut session = GardenSession::open(env.paths().clone(), &clock, Some(42)).unwrap();
        let seed_species = session
            .save()
            .inventory
            .keys()
            .next()
            .cloned()
            .expect("starter inventory has seeds");
        let _ = session.plant(0, &seed_species, clock.now()).unwrap();
    }
    let planted_snap = read_snapshot(env.paths()).unwrap().unwrap();
    assert_eq!(planted_snap.glyph, "·");
    assert_eq!(planted_snap.updated_at, t0);

    // 2. At `+59 s` (`< 60 s`), `prompt` stays on the fast path without refreshing `updated_at`.
    clock.advance_secs(59);
    assert_eq!(
        run_prompt_capture(&env, &clock, PromptArgs::default()),
        "·\n"
    );
    assert_eq!(
        read_snapshot(env.paths()).unwrap().unwrap().updated_at,
        t0,
        "fresh (<60 s) prompt must not rewrite snapshot.json"
    );

    // 3. Advance 1 more second (`age == 60 s`) and then 10 days so the plant blooms:
    // `prompt` refreshes the stale snapshot automatically even without `niwa daemon`.
    clock.advance_secs(1);
    assert_eq!(
        run_prompt_capture(&env, &clock, PromptArgs::default()),
        "·\n"
    );
    assert_eq!(
        read_snapshot(env.paths()).unwrap().unwrap().updated_at,
        Timestamp::from_unix_secs(t0.as_unix_secs() + 60),
        "stale (>=60 s) prompt must refresh snapshot.json"
    );

    clock.advance_secs(10 * 86_400);
    let grown_out = run_prompt_capture(&env, &clock, PromptArgs::default());
    assert_eq!(grown_out, "◎\n");
    let grown_snap = read_snapshot(env.paths()).unwrap().unwrap();
    assert_eq!(grown_snap.glyph, "◎");
    assert_eq!(grown_snap.planted_count, 1);
    assert!(
        grown_snap.dormant,
        "after 10 inactive days (> dormant_after_days=7), refreshed snapshot rests at Mature"
    );

    // 4. If `snapshot.json` becomes stale (`+120 s`) while an interactive TUI holds `garden.lock`,
    // `prompt` must NOT block and must return the existing snapshot's glyph immediately.
    clock.advance_secs(120);
    let _held_lock = GardenLock::acquire(env.paths()).unwrap();
    let contended_out = run_prompt_capture(&env, &clock, PromptArgs::default());
    assert_eq!(contended_out, "◎\n");
}

#[test]
fn prompt_styles_honour_cli_flag_config_default_and_ascii_sprite_set() {
    let env = TestEnv::new();
    let now = SystemClock.now();

    let cases = [
        ("◌", "◌\n", "_\n", "🪴\n"),
        ("·", "·\n", ".\n", "🌰\n"),
        ("╷", "╷\n", ",\n", "🌱\n"),
        ("┼", "┼\n", "+\n", "🌿\n"),
        ("◎", "◎\n", "o\n", "🌷\n"),
        ("◈", "◈\n", "*\n", "🌸\n"),
    ];

    for (unicode_glyph, expected_unicode, expected_ascii, expected_emoji) in cases {
        niwa_cmd(&env)
            .args(["config", "set", "sprites", "unicode"])
            .assert()
            .success();
        niwa_cmd(&env)
            .args(["config", "set", "prompt.style", "glyph"])
            .assert()
            .success();

        let snap = Snapshot::new(unicode_glyph, "summary", Season::Spring, "frame", now);
        write_snapshot(env.paths(), &snap).unwrap();

        // Default (`prompt.style = glyph`, `sprites = unicode`)
        let out_u = niwa_cmd(&env).arg("prompt").assert().success();
        assert_eq!(
            String::from_utf8(out_u.get_output().stdout.clone()).unwrap(),
            expected_unicode
        );

        // Switching `sprites = ascii` takes effect immediately even on the fresh snapshot!
        niwa_cmd(&env)
            .args(["config", "set", "sprites", "ascii"])
            .assert()
            .success();
        let out_a = niwa_cmd(&env).arg("prompt").assert().success();
        assert_eq!(
            String::from_utf8(out_a.get_output().stdout.clone()).unwrap(),
            expected_ascii
        );

        // `--style emoji` overrides `prompt.style = glyph` and `sprites = ascii`.
        let out_e = niwa_cmd(&env)
            .args(["prompt", "--style", "emoji"])
            .assert()
            .success();
        assert_eq!(
            String::from_utf8(out_e.get_output().stdout.clone()).unwrap(),
            expected_emoji
        );

        // `prompt.style = emoji` in `config.toml` applies when `--style` is omitted,
        // and `--style glyph` / `--style none` override it on the CLI.
        niwa_cmd(&env)
            .args(["config", "set", "prompt.style", "emoji"])
            .assert()
            .success();
        let out_cfg_e = niwa_cmd(&env).arg("prompt").assert().success();
        assert_eq!(
            String::from_utf8(out_cfg_e.get_output().stdout.clone()).unwrap(),
            expected_emoji
        );

        let out_override_g = niwa_cmd(&env)
            .args(["--no-color", "prompt", "--style=glyph"])
            .assert()
            .success();
        assert_eq!(
            String::from_utf8(out_override_g.get_output().stdout.clone()).unwrap(),
            expected_ascii
        );

        niwa_cmd(&env)
            .args(["config", "set", "prompt.style", "none"])
            .assert()
            .success();
        let out_cfg_none = niwa_cmd(&env).arg("prompt").assert().success();
        assert!(out_cfg_none.get_output().stdout.is_empty());
    }
}

#[test]
fn prompt_ignores_unknown_snapshot_fields_and_rejects_future_schema_version() {
    let env = TestEnv::new();
    let now = SystemClock.now();
    fs::create_dir_all(env.paths().data_dir()).unwrap();

    // Corrupt `garden.json` and hold `garden.lock` so `niwa prompt` MUST decode `snapshot.json`.
    fs::write(env.paths().garden_file(), b"NOT_JSON").unwrap();
    let _held = GardenLock::acquire(env.paths()).unwrap();

    let payload = serde_json::json!({
        "schema_version": 1,
        "glyph": "◈",
        "summary": "1/8 planted · 1 blooming",
        "season": "autumn",
        "compact_frame": "Niwa · Autumn",
        "updated_at": now.as_unix_secs(),
        "dormant": false,
        "planted_count": 1,
        "blooming_count": 1,
        "unknown_top_level": "ignored",
        "unknown_nested": { "badge": 3, "tags": ["cozy", "autumn"] }
    });
    fs::write(
        env.paths().snapshot_file(),
        serde_json::to_string_pretty(&payload).unwrap(),
    )
    .unwrap();

    let ok_glyph = niwa_cmd(&env).arg("prompt").assert().success();
    assert_eq!(
        String::from_utf8(ok_glyph.get_output().stdout.clone()).unwrap(),
        "◈\n"
    );

    let ok_emoji = niwa_cmd(&env)
        .args(["prompt", "--style", "emoji"])
        .assert()
        .success();
    assert_eq!(
        String::from_utf8(ok_emoji.get_output().stdout.clone()).unwrap(),
        "🌸\n"
    );

    // Unsupported future major version fails cleanly with an explicit schema error.
    let mut future_payload = payload;
    future_payload["schema_version"] = serde_json::json!(99);
    fs::write(
        env.paths().snapshot_file(),
        serde_json::to_string_pretty(&future_payload).unwrap(),
    )
    .unwrap();

    let failed = niwa_cmd(&env).arg("prompt").assert().failure();
    let stderr = String::from_utf8(failed.get_output().stderr.clone()).unwrap();
    assert!(
        stderr.contains("99") && stderr.contains("snapshot.json"),
        "expected unsupported future schema error on stderr, got:\n{stderr}"
    );
}

#[test]
fn prompt_prints_valid_tmux_and_starship_snippets_and_help_lists_prompt() {
    let env = TestEnv::new();

    // Top-level `--help` lists `prompt`.
    let help = niwa_cmd(&env).arg("--help").assert().success();
    let help_text = String::from_utf8(help.get_output().stdout.clone()).unwrap();
    assert!(
        help_text.contains("prompt"),
        "missing `prompt` in --help:\n{help_text}"
    );

    // `niwa prompt --tmux` and `niwa prompt --tmux --style emoji`
    let tmux_default = niwa_cmd(&env).args(["prompt", "--tmux"]).assert().success();
    let tmux_text = String::from_utf8(tmux_default.get_output().stdout.clone()).unwrap();
    assert!(tmux_text.contains("set -g status-interval 60"));
    assert!(tmux_text.contains("#(niwa prompt)"));

    let tmux_emoji = niwa_cmd(&env)
        .args(["prompt", "--tmux", "--style", "emoji"])
        .assert()
        .success();
    let tmux_emoji_text = String::from_utf8(tmux_emoji.get_output().stdout.clone()).unwrap();
    assert!(tmux_emoji_text.contains("#(niwa prompt --style emoji)"));

    // `niwa prompt --starship` and `niwa prompt --starship --style emoji`
    let starship_default = niwa_cmd(&env)
        .args(["prompt", "--starship"])
        .assert()
        .success();
    let starship_text = String::from_utf8(starship_default.get_output().stdout.clone()).unwrap();
    for expected_line in [
        "[custom.niwa]",
        "command = \"niwa prompt\"",
        "when = true",
        "format = \"[$output ]($style)\"",
    ] {
        assert!(
            starship_text
                .lines()
                .any(|line| line.trim() == expected_line),
            "starship snippet missing `{expected_line}`:\n{starship_text}"
        );
    }

    let starship_emoji = niwa_cmd(&env)
        .args(["prompt", "--starship", "--style", "emoji"])
        .assert()
        .success();
    let starship_emoji_text =
        String::from_utf8(starship_emoji.get_output().stdout.clone()).unwrap();
    assert!(
        starship_emoji_text
            .lines()
            .any(|line| line.trim() == "command = \"niwa prompt --style emoji\""),
        "starship emoji snippet missing emoji command:\n{starship_emoji_text}"
    );
}
