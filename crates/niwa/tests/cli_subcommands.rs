#[path = "../../../tests/support/mod.rs"]
mod support;

use assert_cmd::Command;
use serde_json::Value;
use support::TestEnv;
use tg_core::clock::Timestamp;
use tg_core::sim::Garden;
use tg_store::save::{GardenSave, GardenStore};

fn niwa_cmd(env: &TestEnv) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_niwa"));
    cmd.envs(env.env_vars());
    cmd
}

#[test]
fn help_lists_all_mvp_subcommands_and_global_no_color_flag() {
    let env = TestEnv::new();
    let assert = niwa_cmd(&env).arg("--help").assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 help");

    for token in ["peek", "status", "config", "reset", "--no-color"] {
        assert!(
            stdout.contains(token),
            "expected `--help` output to mention `{token}`, got:\n{stdout}"
        );
    }
}

#[test]
fn peek_initializes_starter_garden_persists_snapshot_and_honours_no_color_and_ascii() {
    let env = TestEnv::new();
    assert!(!env.paths().garden_file().exists());
    assert!(!env.paths().snapshot_file().exists());

    // Configure truecolor first so we can verify `--no-color` and `NO_COLOR=1` strip ANSI SGR codes.
    niwa_cmd(&env)
        .args(["config", "set", "color", "truecolor"])
        .assert()
        .success();

    let colored = niwa_cmd(&env)
        .args(["peek", "--width", "80", "--height", "24"])
        .assert()
        .success();
    let colored_out =
        String::from_utf8(colored.get_output().stdout.clone()).expect("utf8 peek output");
    assert!(
        colored_out.contains("\x1b["),
        "expected ANSI escape sequences when color=truecolor, got:\n{colored_out}"
    );
    assert!(env.paths().garden_file().exists());
    assert!(env.paths().snapshot_file().exists());

    // `--no-color` global flag must suppress all ANSI sequences.
    let plain_flag = niwa_cmd(&env)
        .args(["--no-color", "peek", "--width", "80", "--height", "24"])
        .assert()
        .success();
    let plain_flag_out =
        String::from_utf8(plain_flag.get_output().stdout.clone()).expect("utf8 plain peek");
    assert!(
        !plain_flag_out.contains("\x1b["),
        "`--no-color` must emit zero ANSI sequences, got:\n{plain_flag_out}"
    );

    // `NO_COLOR=1` env var must also suppress all ANSI sequences.
    let plain_env = niwa_cmd(&env)
        .env("NO_COLOR", "1")
        .args(["peek", "--width", "80", "--height", "24"])
        .assert()
        .success();
    let plain_env_out =
        String::from_utf8(plain_env.get_output().stdout.clone()).expect("utf8 NO_COLOR peek");
    assert!(
        !plain_env_out.contains("\x1b["),
        "`NO_COLOR=1` must emit zero ANSI sequences, got:\n{plain_env_out}"
    );

    // `--ascii` renders pure 7-bit printable ASCII lines.
    let ascii = niwa_cmd(&env)
        .args([
            "--no-color",
            "peek",
            "--ascii",
            "--width",
            "80",
            "--height",
            "24",
        ])
        .assert()
        .success();
    let ascii_out = String::from_utf8(ascii.get_output().stdout.clone()).expect("utf8 ascii peek");
    assert!(
        ascii_out
            .bytes()
            .all(|b| b == b'\n' || (0x20..=0x7E).contains(&b)),
        "`niwa peek --ascii --no-color` must output strictly 7-bit ASCII, got:\n{ascii_out}"
    );
}

#[test]
fn status_json_and_human_output_report_starter_inventory_and_schema() {
    let env = TestEnv::new();

    let json_assert = niwa_cmd(&env).args(["status", "--json"]).assert().success();
    let raw_json =
        String::from_utf8(json_assert.get_output().stdout.clone()).expect("utf8 status json");
    let parsed: Value = serde_json::from_str(&raw_json).expect("valid status --json payload");

    assert_eq!(parsed["schema_version"].as_u64(), Some(1));
    assert!(parsed["season"].as_str().is_some());
    assert!(parsed["glyph"].as_str().is_some());
    assert_eq!(parsed["dormant"].as_bool(), Some(false));
    assert_eq!(parsed["planted_count"].as_u64(), Some(0));
    assert_eq!(parsed["blooming_count"].as_u64(), Some(0));
    assert_eq!(parsed["plot_count"].as_u64(), Some(9));
    assert_eq!(parsed["plots"].as_array().map(Vec::len), Some(9));

    // Starter garden must provide starter common seeds so a brand-new player can plant immediately.
    let inventory = parsed["inventory"]
        .as_object()
        .expect("inventory object in status --json");
    let total_starter_seeds: u64 = inventory.values().filter_map(Value::as_u64).sum();
    assert!(
        total_starter_seeds >= 4,
        "expected starter seed pouch to contain common seeds, got: {inventory:?}"
    );
    assert!(inventory.contains_key("clover"));

    // Human-readable `niwa status` also succeeds and shows key summary lines.
    let human_assert = niwa_cmd(&env).arg("status").assert().success();
    let human_out =
        String::from_utf8(human_assert.get_output().stdout.clone()).expect("utf8 status");
    assert!(human_out.contains("Niwa"));
    assert!(human_out.contains("Plots:"));
    assert!(human_out.contains("Seeds:"));
}

#[test]
fn config_get_set_list_and_validation_errors_round_trip() {
    let env = TestEnv::new();

    // Default `fps` before config.toml exists is 4.
    let get_default = niwa_cmd(&env)
        .args(["config", "get", "fps"])
        .assert()
        .success();
    assert_eq!(
        String::from_utf8_lossy(&get_default.get_output().stdout).trim(),
        "4"
    );

    // Valid `config set` persists to `config.toml`.
    niwa_cmd(&env)
        .args(["config", "set", "fps", "2"])
        .assert()
        .success();
    niwa_cmd(&env)
        .args(["config", "set", "sprites", "ascii"])
        .assert()
        .success();
    niwa_cmd(&env)
        .args(["config", "set", "hemisphere", "south"])
        .assert()
        .success();

    let get_fps = niwa_cmd(&env)
        .args(["config", "get", "fps"])
        .assert()
        .success();
    assert_eq!(
        String::from_utf8_lossy(&get_fps.get_output().stdout).trim(),
        "2"
    );

    let list_assert = niwa_cmd(&env).args(["config", "list"]).assert().success();
    let list_out =
        String::from_utf8(list_assert.get_output().stdout.clone()).expect("utf8 config list");
    assert!(list_out.contains("fps = 2"));
    assert!(list_out.contains("sprites = ascii"));
    assert!(list_out.contains("hemisphere = south"));

    let path_assert = niwa_cmd(&env).args(["config", "path"]).assert().success();
    let path_out = String::from_utf8_lossy(&path_assert.get_output().stdout);
    assert_eq!(path_out.trim(), env.paths().config_file().to_string_lossy());

    // Out-of-budget fps (> 4 per PLAN §7) must fail with a non-zero exit code and clear message.
    let bad_fps = niwa_cmd(&env)
        .args(["config", "set", "fps", "60"])
        .assert()
        .failure();
    let bad_fps_err = String::from_utf8_lossy(&bad_fps.get_output().stderr);
    assert!(
        bad_fps_err.contains("fps"),
        "expected stderr to mention invalid fps, got: {bad_fps_err}"
    );

    // Unknown key must fail with a non-zero exit code.
    niwa_cmd(&env)
        .args(["config", "get", "nonexistent.setting"])
        .assert()
        .failure();
}

#[test]
fn reset_respects_confirmation_preserves_config_by_default_and_deletes_all_with_flag() {
    let env = TestEnv::new();

    // Initialize garden + custom config.
    niwa_cmd(&env)
        .args(["config", "set", "sprites", "ascii"])
        .assert()
        .success();
    niwa_cmd(&env).args(["status", "--json"]).assert().success();
    assert!(env.paths().garden_file().exists());
    assert!(env.paths().snapshot_file().exists());
    assert!(env.paths().config_file().exists());

    // Answering "n\n" on stdin must abort without deleting garden.json.
    niwa_cmd(&env)
        .arg("reset")
        .write_stdin("n\n")
        .assert()
        .success();
    assert!(
        env.paths().garden_file().exists(),
        "declining confirmation must keep garden.json intact"
    );

    // Answering "y\n" on stdin deletes garden/snapshot while keeping config.toml.
    niwa_cmd(&env)
        .arg("reset")
        .write_stdin("y\n")
        .assert()
        .success();
    assert!(!env.paths().garden_file().exists());
    assert!(!env.paths().snapshot_file().exists());
    assert!(
        env.paths().config_file().exists(),
        "plain `niwa reset` must preserve config.toml"
    );

    // Re-create state, then run `niwa reset --all --yes` to delete both data and config.
    niwa_cmd(&env).args(["status", "--json"]).assert().success();
    assert!(env.paths().garden_file().exists());
    niwa_cmd(&env)
        .args(["reset", "--all", "--yes"])
        .assert()
        .success();
    assert!(!env.paths().garden_file().exists());
    assert!(!env.paths().garden_backup_file().exists());
    assert!(!env.paths().snapshot_file().exists());
    assert!(!env.paths().config_file().exists());
}

#[test]
fn corrupt_save_restores_from_backup_and_emits_recovery_notice_on_stderr() {
    let env = TestEnv::new();
    let store = GardenStore::new(env.paths().clone());

    let t0 = Timestamp::from_unix_secs(1_745_000_000);
    let mut garden = Garden::new(99, 9, t0);
    garden.plant_seed(0, "clover").expect("plant clover");
    let mut save = GardenSave::from_garden(&garden);
    save.inventory.insert("daisy".to_string(), 2);

    // Two saves ensure `garden.json.bak` is written alongside `garden.json`.
    store.save(&save).expect("initial save");
    store.save(&save).expect("second save creates .bak");
    assert!(env.paths().garden_backup_file().exists());

    // Stage corrupt JSON fixture into `garden.json` via `TestEnv` (`tg-store` fixture stager).
    let _ = env.stage_save_fixture("corrupt_truncated.json");

    let assert = niwa_cmd(&env).args(["status", "--json"]).assert().success();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(
        stderr.contains("Restored garden save"),
        "expected backup recovery notice on stderr, got: {stderr}"
    );

    let parsed: Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("valid recovered status json");
    assert_eq!(parsed["planted_count"].as_u64(), Some(1));
    assert_eq!(parsed["inventory"]["daisy"].as_u64(), Some(2));
}
