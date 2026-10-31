#[path = "../../../tests/support/mod.rs"]
mod support;

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use support::TestEnv;
use tg_core::clock::{Clock, FakeClock, Timestamp};
use tg_core::product::{CLI_BIN, DISPLAY_NAME};
use tg_core::season::{Hemisphere, Season, SeasonMode};
use tg_store::config::{
    AppConfig, ColorSetting, ConfigStore, PromptStyle, SignalKind, SpriteSet, DEFAULT_FPS,
};
use tg_store::error::StoreError;
use tg_store::paths::{NiwaPaths, PlatformKind};
use tg_store::snapshot::{
    read_snapshot, write_snapshot, Snapshot, CURRENT_SNAPSHOT_SCHEMA_VERSION,
};

fn env_lookup(vars: &[(&str, OsString)]) -> impl FnMut(&str) -> Option<OsString> {
    let map: HashMap<String, OsString> = vars
        .iter()
        .map(|(k, v)| ((*k).to_string(), v.clone()))
        .collect();
    move |key: &str| map.get(key).cloned()
}

#[test]
fn niwa_home_overrides_platform_paths_on_macos_and_linux() {
    let env = TestEnv::new();
    let vars = [
        ("NIWA_HOME", env.niwa_home().as_os_str().to_os_string()),
        ("HOME", env.home_dir().as_os_str().to_os_string()),
        (
            "XDG_STATE_HOME",
            env.xdg_state_home().as_os_str().to_os_string(),
        ),
        (
            "XDG_CONFIG_HOME",
            env.xdg_config_home().as_os_str().to_os_string(),
        ),
    ];

    for platform in [PlatformKind::MacOs, PlatformKind::Linux] {
        let resolved = NiwaPaths::resolve_from_env(platform, env_lookup(&vars), None)
            .expect("NIWA_HOME must resolve");
        assert_eq!(resolved.data_dir(), env.niwa_home());
        assert_eq!(resolved.config_dir(), env.niwa_home());
        assert_eq!(resolved.garden_file(), env.niwa_home().join("garden.json"));
        assert_eq!(resolved.config_file(), env.niwa_home().join("config.toml"));
    }
}

#[test]
fn macos_paths_use_application_support_display_name() {
    let env = TestEnv::new();
    let vars = [("HOME", env.home_dir().as_os_str().to_os_string())];

    let resolved = NiwaPaths::resolve_from_env(PlatformKind::MacOs, env_lookup(&vars), None)
        .expect("macOS path resolution must succeed");

    let expected = env
        .home_dir()
        .join("Library")
        .join("Application Support")
        .join(DISPLAY_NAME);
    assert_eq!(resolved.data_dir(), expected);
    assert_eq!(resolved.config_dir(), expected);
    assert_eq!(resolved.garden_file(), expected.join("garden.json"));
    assert_eq!(
        resolved.garden_backup_file(),
        expected.join("garden.json.bak")
    );
    assert_eq!(resolved.lock_file(), expected.join("garden.lock"));
    assert_eq!(resolved.snapshot_file(), expected.join("snapshot.json"));
    assert_eq!(resolved.config_file(), expected.join("config.toml"));
}

#[test]
fn linux_paths_use_xdg_state_and_config_with_overrides_and_defaults() {
    let env = TestEnv::new();

    // 1. Explicit XDG_STATE_HOME + XDG_CONFIG_HOME
    let xdg_vars = [
        ("HOME", env.home_dir().as_os_str().to_os_string()),
        (
            "XDG_STATE_HOME",
            env.xdg_state_home().as_os_str().to_os_string(),
        ),
        (
            "XDG_CONFIG_HOME",
            env.xdg_config_home().as_os_str().to_os_string(),
        ),
    ];
    let custom = NiwaPaths::resolve_from_env(PlatformKind::Linux, env_lookup(&xdg_vars), None)
        .expect("Linux XDG override paths must resolve");
    assert_eq!(custom.data_dir(), env.xdg_state_home().join(CLI_BIN));
    assert_eq!(
        custom.config_file(),
        env.xdg_config_home().join(CLI_BIN).join("config.toml")
    );

    // 2. Fallback to ~/.local/state/niwa and ~/.config/niwa/config.toml when XDG vars are unset/empty
    let default_vars = [
        ("HOME", env.home_dir().as_os_str().to_os_string()),
        ("XDG_STATE_HOME", OsString::new()),
    ];
    let defaults =
        NiwaPaths::resolve_from_env(PlatformKind::Linux, env_lookup(&default_vars), None)
            .expect("Linux XDG default paths must resolve");
    assert_eq!(
        defaults.data_dir(),
        env.home_dir().join(".local").join("state").join(CLI_BIN)
    );
    assert_eq!(
        defaults.config_file(),
        env.home_dir()
            .join(".config")
            .join(CLI_BIN)
            .join("config.toml")
    );

    // 3. Fallback home from `directories::BaseDirs` when HOME env var is absent
    let empty_vars: [(&str, OsString); 0] = [];
    let from_fallback = NiwaPaths::resolve_from_env(
        PlatformKind::MacOs,
        env_lookup(&empty_vars),
        Some(env.home_dir()),
    )
    .expect("fallback home directory must resolve");
    assert_eq!(
        from_fallback.data_dir(),
        env.home_dir()
            .join("Library")
            .join("Application Support")
            .join(DISPLAY_NAME)
    );

    // 4. Error when neither NIWA_HOME, HOME, nor fallback home is available
    let err = NiwaPaths::resolve_from_env(PlatformKind::Linux, env_lookup(&empty_vars), None)
        .expect_err("missing home must fail explicitly");
    assert!(matches!(err, StoreError::MissingHomeDirectory));
}

#[test]
fn config_round_trips_all_section_6_keys_and_enforces_validation() {
    let env = TestEnv::new();
    let clock = FakeClock::from_unix_secs(1_730_000_000);
    let store = ConfigStore::new(env.paths().clone());

    // Missing config.toml returns sensible PLAN defaults without touching disk.
    let mut cfg = store.load().expect("default config load");
    assert_eq!(cfg, AppConfig::default());
    assert_eq!(cfg.fps, DEFAULT_FPS);
    assert_eq!(cfg.get_key("sprites").unwrap(), "unicode");
    assert_eq!(cfg.get_key("color").unwrap(), "auto");
    assert_eq!(cfg.get_key("fps").unwrap(), "4");
    assert_eq!(cfg.get_key("season_mode").unwrap(), "calendar");
    assert_eq!(cfg.get_key("hemisphere").unwrap(), "north");
    assert_eq!(cfg.get_key("dormant_after_days").unwrap(), "7");
    assert_eq!(cfg.get_key("signals.activity.enabled").unwrap(), "false");
    assert_eq!(
        cfg.get_key("signals.activity.consented_at").unwrap(),
        "none"
    );
    assert_eq!(cfg.get_key("prompt.style").unwrap(), "glyph");
    assert_eq!(cfg.get_key("menubar.show_title").unwrap(), "true");

    // Mutate every single PLAN §6 setting through `set_key` / `set_key_with_timestamp`.
    cfg.set_key("sprites", "ascii").unwrap();
    cfg.set_key("color", "256").unwrap();
    cfg.set_key("fps", "2").unwrap();
    cfg.set_key("season_mode", "garden").unwrap();
    cfg.set_key("hemisphere", "south").unwrap();
    cfg.set_key("dormant_after_days", "14").unwrap();

    cfg.set_key_with_timestamp("signals.activity.enabled", "true", Some(clock.now()))
        .unwrap();
    clock.advance_secs(120);
    cfg.enable_signal(SignalKind::Git, clock.now());
    clock.advance_secs(60);
    cfg.set_key("signals.shell.enabled", "true").unwrap();
    cfg.set_key(
        "signals.shell.consented_at",
        &clock.now().as_unix_secs().to_string(),
    )
    .unwrap();

    let repo_a = env.home_dir().join("work").join("garden-alpha");
    let repo_b = env.home_dir().join("work").join("garden beta");
    let repos_csv = format!("{}, {}", repo_a.display(), repo_b.display());
    cfg.set_key("git.repos", &repos_csv).unwrap();
    cfg.set_key("git.author_email", "gardener@local.invalid")
        .unwrap();
    cfg.set_key("prompt.style", "emoji").unwrap();
    cfg.set_key("menubar.show_title", "false").unwrap();

    store.save(&cfg).expect("save config.toml");
    assert!(env.paths().config_file().is_file());

    // Reload from disk and verify full fidelity + unknown-field tolerance.
    let mut raw_toml = fs::read_to_string(env.paths().config_file()).unwrap();
    raw_toml.push_str("\nfuture_top_level_hint = \"ignored\"\n");
    fs::write(env.paths().config_file(), raw_toml).unwrap();

    let loaded = store.load().expect("reload config.toml");
    assert_eq!(loaded.sprites, SpriteSet::Ascii);
    assert_eq!(loaded.color, ColorSetting::Ansi256);
    assert_eq!(loaded.fps, 2);
    assert_eq!(loaded.season_mode, SeasonMode::Garden);
    assert_eq!(loaded.hemisphere, Hemisphere::South);
    assert_eq!(loaded.dormant_after_days, 14);
    assert!(loaded.signals.activity.enabled);
    assert_eq!(
        loaded.signals.activity.consented_at,
        Some(Timestamp::from_unix_secs(1_730_000_000))
    );
    assert!(loaded.signals.git.enabled);
    assert_eq!(
        loaded.signals.git.consented_at,
        Some(Timestamp::from_unix_secs(1_730_000_120))
    );
    assert!(loaded.signals.shell.enabled);
    assert_eq!(
        loaded.signals.shell.consented_at,
        Some(Timestamp::from_unix_secs(1_730_000_180))
    );
    assert_eq!(loaded.git.repos, vec![repo_a, repo_b]);
    assert_eq!(
        loaded.git.author_email.as_deref(),
        Some("gardener@local.invalid")
    );
    assert_eq!(loaded.prompt.style, PromptStyle::Emoji);
    assert!(!loaded.menubar.show_title);

    // Reject out-of-budget FPS (> 4 or 0), zero dormant days, and unknown keys.
    let mut bad = loaded.clone();
    assert!(matches!(
        bad.set_key("fps", "5"),
        Err(StoreError::InvalidConfigValue { .. })
    ));
    assert!(matches!(
        bad.set_key("fps", "0"),
        Err(StoreError::InvalidConfigValue { .. })
    ));
    assert!(matches!(
        bad.set_key("dormant_after_days", "0"),
        Err(StoreError::InvalidConfigValue { .. })
    ));
    assert!(matches!(
        bad.set_key("unknown.setting", "1"),
        Err(StoreError::InvalidConfigKey { .. })
    ));

    // Direct file edit with fps = 10 is also caught on load().
    fs::write(env.paths().config_file(), "fps = 10\n").unwrap();
    assert!(matches!(
        store.load(),
        Err(StoreError::InvalidConfigValue { .. })
    ));

    // Ensure JSON array input also works for `git.repos` (used when paths contain commas).
    let mut comma_cfg = AppConfig::default();
    comma_cfg
        .set_key("git.repos", "[\"/tmp/path,one\", \"/tmp/path_two\"]")
        .unwrap();
    assert_eq!(
        comma_cfg.git.repos,
        vec![
            PathBuf::from("/tmp/path,one"),
            PathBuf::from("/tmp/path_two")
        ]
    );
}

#[test]
fn snapshot_writer_is_atomic_golden_and_tolerates_unknown_fields() {
    let env = TestEnv::new();
    let clock = FakeClock::from_unix_secs(1_730_001_200);

    assert_eq!(read_snapshot(env.paths()).expect("empty snapshot"), None);

    let mut snap = Snapshot::new(
        "✿",
        "2 blooming · 3 planted (autumn)",
        Season::Autumn,
        "[✿][🌿][🌱][·]",
        clock.now(),
    );
    snap.dormant = false;
    snap.planted_count = 3;
    snap.blooming_count = 2;

    write_snapshot(env.paths(), &snap).expect("write snapshot.json");
    let raw = fs::read_to_string(env.paths().snapshot_file()).expect("read snapshot.json");
    insta::assert_snapshot!("snapshot_json_v1_golden", raw);

    let decoded = read_snapshot(env.paths())
        .expect("read valid snapshot")
        .expect("snapshot exists");
    assert_eq!(decoded, snap);
    assert_eq!(decoded.schema_version, CURRENT_SNAPSHOT_SCHEMA_VERSION);

    // Inject unknown future minor fields and confirm tolerant decoding (PLAN §9 & T13).
    let mut value: serde_json::Value = serde_json::from_str(&raw).unwrap();
    value["ambient_particle"] = serde_json::json!("falling_leaves");
    value["extra_metrics"] = serde_json::json!({ "wind": 2 });
    fs::write(
        env.paths().snapshot_file(),
        serde_json::to_string_pretty(&value).unwrap(),
    )
    .unwrap();

    let tolerant = read_snapshot(env.paths())
        .expect("unknown fields must be ignored")
        .expect("snapshot exists");
    assert_eq!(tolerant, snap);

    // Reject unsupported future major schema_version cleanly.
    value["schema_version"] = serde_json::json!(99);
    fs::write(
        env.paths().snapshot_file(),
        serde_json::to_string_pretty(&value).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        read_snapshot(env.paths()),
        Err(StoreError::UnsupportedFutureSchema { found: 99, .. })
    ));
}
