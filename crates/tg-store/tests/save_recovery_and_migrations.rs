#[path = "../../../tests/support/mod.rs"]
mod support;

use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use support::TestEnv;
use tg_core::balance::Balance;
use tg_core::catch_up::{CatchUpEvent, CatchUpOptions};
use tg_core::clock::{Clock, FakeClock, Timestamp};
use tg_core::sim::{Garden, TickInput};
use tg_core::species::SpeciesCatalog;
use tg_store::atomic::FaultPoint;
use tg_store::error::StoreError;
use tg_store::migrate::{MigrationRunner, SaveMigration, CURRENT_SAVE_SCHEMA_VERSION};
use tg_store::save::{AlmanacEntry, GardenSave, GardenStore, RecoveryNotice};

#[test]
fn loads_minimal_v1_fixture_and_round_trips_full_garden_state() {
    let env = TestEnv::new();
    let clock = FakeClock::from_unix_secs(1_700_000_000);
    let balance = Balance::load_default();
    let store = GardenStore::new(env.paths().clone());

    assert_eq!(store.load().expect("fresh store has no save"), None);

    let _ = env.stage_save_fixture("minimal_v1.json");
    let loaded = store
        .load()
        .expect("load minimal_v1.json")
        .expect("save must exist");
    assert_eq!(loaded.notice, None);
    assert_eq!(loaded.migrated_from, None);
    assert_eq!(loaded.save.schema_version, CURRENT_SAVE_SCHEMA_VERSION);
    assert_eq!(loaded.save.seed, 42);
    assert_eq!(loaded.save.last_tick_at, clock.now());

    // Build a richer garden state, persist twice (creating .bak), and verify round-trip.
    let mut garden = Garden::from_clock(777, &balance, &clock);
    garden.plant_seed(0, "sun_moss").unwrap();
    garden.plant_seed(2, "moon_fern").unwrap();
    garden.tick_n(5, TickInput::active(), &balance);
    clock.advance_ticks(5);

    let mut save_v1 = GardenSave::from_garden(&garden);
    save_v1.inventory.insert("sun_moss".to_string(), 3);
    save_v1.almanac.insert(
        "sun_moss".to_string(),
        AlmanacEntry {
            discovered_at: Timestamp::from_unix_secs(1_700_000_000),
            first_bloomed_at: None,
            harvest_count: 0,
        },
    );
    save_v1.diagnostics.dropped_future_events = 2;

    store.save(&save_v1).expect("first save");
    assert!(
        env.paths().garden_backup_file().is_file(),
        "saving over staged minimal_v1.json must rotate previous valid save into garden.json.bak"
    );

    let reloaded = store
        .load()
        .expect("reload saved garden")
        .expect("save present");
    assert_eq!(reloaded.notice, None);
    assert_eq!(reloaded.save, save_v1);
    assert_eq!(reloaded.save.to_garden(), garden);
}

#[test]
fn aborted_writes_at_every_fault_point_leave_previous_save_loadable() {
    let env = TestEnv::new();
    let clock = FakeClock::from_unix_secs(1_710_000_000);
    let balance = Balance::load_default();
    let store = GardenStore::new(env.paths().clone());

    // 1. Aborted initial write leaves no corrupt garden.json behind.
    let initial_garden = Garden::from_clock(99, &balance, &clock);
    let initial_save = GardenSave::from_garden(&initial_garden);
    let err = store
        .save_with_fault(&initial_save, Some(FaultPoint::AfterTempPartialWrite))
        .expect_err("fault point must abort initial write");
    assert!(matches!(err, StoreError::AbortedWrite { .. }));
    assert!(!env.paths().garden_file().exists());
    assert_eq!(store.load().expect("load after aborted first write"), None);

    // 2. Commit a valid baseline save S1.
    store.save(&initial_save).expect("commit baseline S1");
    let s1_bytes = fs::read(env.paths().garden_file()).expect("read S1 bytes");

    // 3. Prepare modified save S2 and abort at both FaultPoints.
    let mut next_garden = initial_garden.clone();
    next_garden.plant_seed(0, "sun_moss").unwrap();
    next_garden.tick_n(3, TickInput::active(), &balance);
    let mut s2 = GardenSave::from_garden(&next_garden);
    s2.inventory.insert("frost_lotus".to_string(), 9);

    for fault in [FaultPoint::AfterTempPartialWrite, FaultPoint::BeforeRename] {
        let abort_err = store
            .save_with_fault(&s2, Some(fault))
            .expect_err("injected fault must abort save");
        assert!(matches!(abort_err, StoreError::AbortedWrite { .. }));

        // Primary garden.json must remain byte-for-byte identical to S1 and load cleanly.
        let disk_bytes = fs::read(env.paths().garden_file()).expect("read garden.json after abort");
        assert_eq!(disk_bytes, s1_bytes);

        let loaded = store
            .load()
            .expect("load after aborted write must succeed")
            .expect("baseline S1 must still exist");
        assert_eq!(loaded.notice, None);
        assert_eq!(loaded.save, initial_save);
    }

    // 4. Subsequent normal save S2 succeeds even with leftover orphan .tmp files present.
    store.save(&s2).expect("normal save after aborts");
    let final_loaded = store.load().unwrap().unwrap();
    assert_eq!(final_loaded.save, s2);
}

#[test]
fn corrupt_save_restores_from_backup_with_notice_and_heals_primary() {
    let env = TestEnv::new();
    let clock = FakeClock::from_unix_secs(1_720_000_000);
    let balance = Balance::load_default();
    let store = GardenStore::new(env.paths().clone());

    let mut g1 = Garden::from_clock(1234, &balance, &clock);
    g1.plant_seed(0, "sun_moss").unwrap();
    let mut s1 = GardenSave::from_garden(&g1);
    s1.inventory.insert("sun_moss".to_string(), 5);
    store.save(&s1).expect("save generation 1");

    g1.tick_n(4, TickInput::active(), &balance);
    let mut s2 = GardenSave::from_garden(&g1);
    s2.inventory.insert("sun_moss".to_string(), 8);
    store
        .save(&s2)
        .expect("save generation 2 (rotates s1 into .bak)");

    // Corrupt primary garden.json with truncated invalid JSON.
    fs::write(
        env.paths().garden_file(),
        b"{\"schema_version\": 1, \"seed\": ",
    )
    .unwrap();

    // Even if a caller mistakenly tried saving s2 before loading, corrupt primary MUST NOT overwrite good .bak!
    // Verify .bak still holds s1 right now by loading first and checking recovery notice (T05 AC 2).
    let recovered = store
        .load()
        .expect("corrupt primary must recover from valid .bak")
        .expect("recovered save must be present");

    assert_eq!(recovered.save, s1);
    let notice = recovered
        .notice
        .expect("recovery from .bak must emit a RecoveryNotice");
    match &notice {
        RecoveryNotice::RestoredFromBackup {
            primary_path,
            backup_path,
            reason,
        } => {
            assert_eq!(primary_path, &env.paths().garden_file());
            assert_eq!(backup_path, &env.paths().garden_backup_file());
            assert!(!reason.is_empty());
        }
    }
    assert!(notice.to_string().contains("Restored garden save"));

    // Primary garden.json on disk must now be healed so the next load emits no notice.
    let healed = store
        .load()
        .expect("load healed primary")
        .expect("healed save exists");
    assert_eq!(healed.notice, None);
    assert_eq!(healed.save, s1);
}

#[test]
fn saving_over_corrupt_primary_never_clobbers_valid_backup() {
    let env = TestEnv::new();
    let clock = FakeClock::from_unix_secs(1_720_100_000);
    let balance = Balance::load_default();
    let store = GardenStore::new(env.paths().clone());

    let g1 = Garden::from_clock(55, &balance, &clock);
    let mut s1 = GardenSave::from_garden(&g1);
    s1.inventory.insert("sun_moss".to_string(), 1);
    store.save(&s1).unwrap();

    let mut s2 = s1.clone();
    s2.inventory.insert("sun_moss".to_string(), 2);
    store.save(&s2).unwrap(); // .bak now holds s1

    // Corrupt primary garden.json, then write s3. Because primary is corrupt, .bak must stay s1!
    fs::write(env.paths().garden_file(), b"<<<corrupt>>>").unwrap();
    let mut s3 = s2.clone();
    s3.inventory.insert("sun_moss".to_string(), 3);
    store.save(&s3).unwrap();

    let bak_raw = fs::read_to_string(env.paths().garden_backup_file()).unwrap();
    let bak_parsed: GardenSave =
        serde_json::from_str(&bak_raw).expect(".bak must remain valid JSON");
    assert_eq!(bak_parsed, s1);
}

#[test]
fn unrecoverable_corrupt_save_without_valid_backup_never_silently_resets() {
    let env = TestEnv::new();
    let store = GardenStore::new(env.paths().clone());

    // Case A: Corrupt primary with missing .bak.
    let bad_bytes = b"{not valid json at all";
    fs::write(env.paths().garden_file(), bad_bytes).unwrap();
    let err = store
        .load()
        .expect_err("corrupt primary without .bak must error, never silently reset");
    assert!(matches!(err, StoreError::CorruptSaveUnrecoverable { .. }));
    assert_eq!(
        fs::read(env.paths().garden_file()).unwrap(),
        bad_bytes,
        "unrecoverable corrupt primary must be left untouched for user inspection"
    );

    // Case B: Corrupt primary AND corrupt .bak.
    fs::write(env.paths().garden_backup_file(), b"also broken").unwrap();
    let err_both = store
        .load()
        .expect_err("corrupt primary + corrupt .bak must error");
    assert!(matches!(
        err_both,
        StoreError::CorruptSaveUnrecoverable { .. }
    ));
}

struct TestMigrateV1ToV2;

impl SaveMigration for TestMigrateV1ToV2 {
    fn source_version(&self) -> u32 {
        1
    }

    fn migrate(&self, value: &mut Value) -> Result<(), StoreError> {
        if let Some(obj) = value.as_object_mut() {
            obj.insert("migrated_v2_marker".to_string(), json!(true));
        }
        Ok(())
    }
}

#[test]
fn migration_v0_to_v1_runs_step_by_step_and_matches_golden() {
    let env = TestEnv::new();
    let store = GardenStore::new(env.paths().clone());

    // Stage v0 fixture into garden.json and load through GardenStore.
    let _ = env.stage_save_fixture("sample_v0.json");
    let outcome = store
        .load()
        .expect("v0 save must migrate and load")
        .expect("migrated save present");
    assert_eq!(outcome.migrated_from, Some(0));
    assert_eq!(outcome.notice, None);
    assert_eq!(outcome.save.schema_version, CURRENT_SAVE_SCHEMA_VERSION);
    assert_eq!(outcome.save.seed, 108);
    assert_eq!(outcome.save.tick_index, 10);

    let pretty = serde_json::to_string_pretty(&outcome.save).unwrap();
    insta::assert_snapshot!("migrated_v0_to_v1_golden", pretty);

    // Verify multi-step chaining (v0 -> v1 -> v2) executes one step at a time.
    let raw_v0: Value =
        serde_json::from_str(&env.read_save_fixture("sample_v0.json")).expect("parse sample_v0");
    let runner = MigrationRunner::new(
        2,
        vec![
            Box::new(tg_store::migrate::MigrateV0ToV1),
            Box::new(TestMigrateV1ToV2),
        ],
    );
    let report = runner
        .migrate_value(Path::new("sample_v0.json"), raw_v0)
        .expect("v0 -> v1 -> v2 step chain");
    assert_eq!(report.original_version, 0);
    assert_eq!(report.final_version, 2);
    assert_eq!(report.steps_applied, 2);
    assert_eq!(report.value["schema_version"], json!(2));
    assert_eq!(report.value["migrated_v2_marker"], json!(true));
    assert_eq!(report.value["inventory"], json!({}));

    // Verify UnsupportedFutureSchema is reported directly and NOT treated as corruption that clobbers from .bak!
    store.save(&outcome.save).unwrap(); // writes valid v1 and .bak
    let mut future_val: Value =
        serde_json::from_str(&fs::read_to_string(env.paths().garden_file()).unwrap()).unwrap();
    future_val["schema_version"] = json!(42);
    fs::write(
        env.paths().garden_file(),
        serde_json::to_string_pretty(&future_val).unwrap(),
    )
    .unwrap();

    let future_err = store
        .load()
        .expect_err("future schema_version must not fall back to older .bak");
    assert!(matches!(
        future_err,
        StoreError::UnsupportedFutureSchema { found: 42, .. }
    ));
}

#[test]
fn record_catch_up_accumulates_clock_skew_and_dropped_event_diagnostics() {
    let env = TestEnv::new();
    let balance = Balance::load_default();
    let catalog = SpeciesCatalog::load_embedded().unwrap();
    let clock = FakeClock::from_unix_secs(1_780_000_000);
    let store = GardenStore::new(env.paths().clone());

    let mut garden = Garden::from_clock(88, &balance, &clock);
    garden.plant_seed(0, "daisy").unwrap();
    let mut save = GardenSave::from_garden(&garden);

    // 1. Backwards clock skew increments `clock_skew_events`.
    let skew_out = garden.catch_up(
        clock.now().add_secs(-120),
        CatchUpOptions::default(),
        &balance,
        &catalog,
    );
    save.record_catch_up(&garden, &skew_out);
    assert_eq!(save.diagnostics.clock_skew_events, 1);

    // 2. Forward catch-up with out-of-window events increments dropped event counters.
    let now = clock.advance_ticks(10);
    let events = [
        CatchUpEvent::active_at(Timestamp::from_unix_secs(1_779_999_000)),
        CatchUpEvent::active_at(now.add_secs(600)),
    ];
    let fwd_out =
        garden.catch_up_with_events(now, &events, CatchUpOptions::default(), &balance, &catalog);
    save.record_catch_up(&garden, &fwd_out);
    store.save(&save).unwrap();

    let reloaded = store.load().unwrap().unwrap().save;
    assert_eq!(reloaded.diagnostics.clock_skew_events, 1);
    assert_eq!(reloaded.diagnostics.dropped_pre_creation_events, 1);
    assert_eq!(reloaded.diagnostics.dropped_future_events, 1);
    assert_eq!(reloaded.tick_index, 10);
}
