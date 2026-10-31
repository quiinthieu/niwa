#[path = "../../../tests/support/mod.rs"]
mod support;

use std::fs;
use support::TestEnv;

#[test]
fn test_env_isolates_paths_and_cleans_up_on_drop() {
    let real_home = std::env::var_os("HOME");
    let captured_niwa_home;
    {
        let env = TestEnv::new();
        captured_niwa_home = env.niwa_home().to_path_buf();

        assert!(env.niwa_home().is_dir());
        assert!(env.home_dir().is_dir());
        assert!(env.xdg_config_home().is_dir());
        assert!(env.xdg_state_home().is_dir());

        if let Some(ref real) = real_home {
            assert_ne!(env.home_dir().as_os_str(), real);
            assert_ne!(env.niwa_home().as_os_str(), real);
        }
        assert!(env.niwa_home().starts_with(std::env::temp_dir()));

        assert_eq!(
            env.paths().garden_file(),
            env.niwa_home().join("garden.json")
        );
        assert_eq!(
            env.paths().garden_backup_file(),
            env.niwa_home().join("garden.json.bak")
        );
        assert_eq!(
            env.paths().events_file(),
            env.niwa_home().join("events.jsonl")
        );
        assert_eq!(
            env.paths().snapshot_file(),
            env.niwa_home().join("snapshot.json")
        );
        assert_eq!(
            env.paths().config_file(),
            env.niwa_home().join("config.toml")
        );
        assert_eq!(env.paths().lock_file(), env.niwa_home().join("garden.lock"));
    }

    assert!(
        !captured_niwa_home.exists(),
        "TestEnv temp directory must be removed on Drop"
    );
}

#[test]
fn fixture_loaders_read_and_stage_saves_and_spools() {
    let env = TestEnv::new();

    let raw_save = env.read_save_fixture("minimal_v1.json");
    assert!(raw_save.contains("\"schema_version\": 1"));

    let raw_spool = env.read_spool_fixture("sample_events.jsonl");
    assert_eq!(raw_spool.lines().count(), 3);

    let staged_save = env.stage_save_fixture("minimal_v1.json");
    assert_eq!(staged_save, env.paths().garden_file());
    assert_eq!(
        fs::read_to_string(&staged_save).expect("read staged save"),
        raw_save
    );

    let staged_spool = env.stage_spool_fixture("sample_events.jsonl");
    assert_eq!(staged_spool, env.paths().events_file());
    assert_eq!(
        fs::read_to_string(&staged_spool).expect("read staged spool"),
        raw_spool
    );

    assert!(env.fixtures().read_save("../minimal_v1.json").is_err());
    assert!(env.fixtures().read_spool("nested/events.jsonl").is_err());
}

#[test]
fn test_env_and_fixture_staging_pass_100x_in_a_loop() {
    for _ in 0..100 {
        let env = TestEnv::new();
        let save_path = env.stage_save_fixture("minimal_v1.json");
        let spool_path = env.stage_spool_fixture("sample_events.jsonl");
        assert!(save_path.is_file());
        assert!(spool_path.is_file());
    }
}
