#[path = "../../../tests/support/mod.rs"]
mod support;

use std::fs;
use std::str::FromStr;
use support::TestEnv;
use tg_core::clock::{Clock, FakeClock, Timestamp, TICK_SECS};
use tg_core::season::Season;
use tg_store::config::PromptStyle;
use tg_store::error::StoreError;
use tg_store::snapshot::{
    read_snapshot, write_snapshot, Snapshot, CURRENT_SNAPSHOT_SCHEMA_VERSION, SNAPSHOT_MAX_AGE_SECS,
};

#[test]
fn snapshot_v1_schema_goldens_cover_empty_active_dormant_and_ascii_states() {
    let env = TestEnv::new();
    let clock = FakeClock::from_unix_secs(1_745_000_000);

    // 1. Empty starter garden (Spring, Unicode).
    let empty_snap = Snapshot::new(
        "◌",
        "0/8 planted · 0 blooming",
        Season::Spring,
        "Niwa · Spring · 0/8 planted\n  1. (empty)\n  2. (empty)",
        clock.now(),
    );
    write_snapshot(env.paths(), &empty_snap).expect("write empty snapshot");
    let raw_empty = fs::read_to_string(env.paths().snapshot_file()).expect("read empty snapshot");
    insta::assert_snapshot!("snapshot_v1_empty_starter_garden_golden", raw_empty);
    assert_eq!(
        read_snapshot(env.paths()).unwrap(),
        Some(empty_snap),
        "empty starter snapshot must round-trip cleanly"
    );

    // 2. Active multi-plant blooming garden (Summer, Unicode).
    let mut active_snap = Snapshot::new(
        "◈",
        "4/8 planted · 1 blooming",
        Season::Summer,
        "Niwa · Summer · 4/8 planted · 1 blooming\n  1. ◈ Moss (Blooming)\n  2. ◎ Fern (Mature)",
        Timestamp::from_unix_secs(1_752_000_060),
    );
    active_snap.planted_count = 4;
    active_snap.blooming_count = 1;
    write_snapshot(env.paths(), &active_snap).expect("write active snapshot");
    let raw_active = fs::read_to_string(env.paths().snapshot_file()).expect("read active snapshot");
    insta::assert_snapshot!("snapshot_v1_active_blooming_garden_golden", raw_active);
    assert_eq!(
        read_snapshot(env.paths()).unwrap(),
        Some(active_snap),
        "active blooming snapshot must round-trip cleanly"
    );

    // 3. Dormant resting garden (Winter, Unicode).
    let mut dormant_snap = Snapshot::new(
        "◎",
        "2/8 planted · 0 blooming · resting",
        Season::Winter,
        "Niwa · Winter · 2/8 planted · resting\n  1. ◎ Pine (Mature · resting)\n  2. ╷ Clover (Sprout · resting)",
        Timestamp::from_unix_secs(1_766_000_120),
    );
    dormant_snap.dormant = true;
    dormant_snap.planted_count = 2;
    dormant_snap.blooming_count = 0;
    write_snapshot(env.paths(), &dormant_snap).expect("write dormant snapshot");
    let raw_dormant =
        fs::read_to_string(env.paths().snapshot_file()).expect("read dormant snapshot");
    insta::assert_snapshot!("snapshot_v1_dormant_winter_garden_golden", raw_dormant);
    assert_eq!(
        read_snapshot(env.paths()).unwrap(),
        Some(dormant_snap),
        "dormant winter snapshot must round-trip cleanly"
    );

    // 4. ASCII sprite-set garden (Autumn, 7-bit ASCII).
    let mut ascii_snap = Snapshot::new(
        "*",
        "3/8 planted · 2 blooming",
        Season::Autumn,
        "Niwa - Autumn - 3/8 planted - 2 blooming\n  1. * Maple (Blooming)\n  2. + Sage (Young)",
        Timestamp::from_unix_secs(1_760_500_180),
    );
    ascii_snap.planted_count = 3;
    ascii_snap.blooming_count = 2;
    write_snapshot(env.paths(), &ascii_snap).expect("write ascii snapshot");
    let raw_ascii = fs::read_to_string(env.paths().snapshot_file()).expect("read ascii snapshot");
    insta::assert_snapshot!("snapshot_v1_ascii_mode_garden_golden", raw_ascii);
    assert_eq!(
        read_snapshot(env.paths()).unwrap(),
        Some(ascii_snap),
        "ascii snapshot must round-trip cleanly"
    );
}

#[test]
fn snapshot_decoder_ignores_unknown_fields_defaults_optional_fields_and_guards_versions() {
    let env = TestEnv::new();
    let ts = Timestamp::from_unix_secs(1_745_001_000);

    // Unknown top-level and deeply nested JSON fields across all JSON value kinds must be ignored.
    let forward_compatible_payload = serde_json::json!({
        "schema_version": CURRENT_SNAPSHOT_SCHEMA_VERSION,
        "glyph": "◈",
        "summary": "2/8 planted · 1 blooming",
        "season": "spring",
        "compact_frame": "Niwa · Spring",
        "updated_at": ts.as_unix_secs(),
        "dormant": false,
        "planted_count": 2,
        "blooming_count": 1,
        "future_string": "sakura_petals",
        "future_bool": true,
        "future_number": 42.5,
        "future_null": null,
        "future_array": ["petal", "firefly", {"x": 3, "y": 1}],
        "future_object": {
            "menubar_badge": "bloom",
            "nested": { "version": 2, "enabled": false }
        }
    });
    fs::create_dir_all(env.paths().data_dir()).unwrap();
    fs::write(
        env.paths().snapshot_file(),
        serde_json::to_string_pretty(&forward_compatible_payload).unwrap(),
    )
    .unwrap();

    let decoded = read_snapshot(env.paths())
        .expect("unknown fields must be ignored")
        .expect("snapshot exists");
    assert_eq!(decoded.schema_version, 1);
    assert_eq!(decoded.glyph, "◈");
    assert_eq!(decoded.season, Season::Spring);
    assert_eq!(decoded.updated_at, ts);
    assert!(!decoded.dormant);
    assert_eq!(decoded.planted_count, 2);
    assert_eq!(decoded.blooming_count, 1);

    // Minimal v1 payload omitting `#[serde(default)]` fields (`dormant`, `planted_count`, `blooming_count`).
    let minimal_v1 = serde_json::json!({
        "schema_version": 1,
        "glyph": "◌",
        "summary": "0/8 planted · 0 blooming",
        "season": "autumn",
        "compact_frame": "empty",
        "updated_at": ts.as_unix_secs(),
        "unknown_extra": "ignored"
    });
    fs::write(
        env.paths().snapshot_file(),
        serde_json::to_string_pretty(&minimal_v1).unwrap(),
    )
    .unwrap();
    let minimal_decoded = read_snapshot(env.paths())
        .expect("minimal v1 payload must decode")
        .expect("snapshot exists");
    assert!(!minimal_decoded.dormant);
    assert_eq!(minimal_decoded.planted_count, 0);
    assert_eq!(minimal_decoded.blooming_count, 0);

    // Invalid / zero / missing schema_version -> CorruptSave.
    for bad_version in [
        serde_json::json!(0),
        serde_json::json!("1"),
        serde_json::json!(null),
    ] {
        let mut bad = minimal_v1.clone();
        bad["schema_version"] = bad_version;
        fs::write(
            env.paths().snapshot_file(),
            serde_json::to_string_pretty(&bad).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            read_snapshot(env.paths()),
            Err(StoreError::CorruptSave { .. })
        ));
    }

    // Future major schema_version -> UnsupportedFutureSchema.
    for future_ver in [2_u32, 99_u32] {
        let mut future = minimal_v1.clone();
        future["schema_version"] = serde_json::json!(future_ver);
        fs::write(
            env.paths().snapshot_file(),
            serde_json::to_string_pretty(&future).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            read_snapshot(env.paths()),
            Err(StoreError::UnsupportedFutureSchema {
                kind: "snapshot.json",
                found,
                max_supported: CURRENT_SNAPSHOT_SCHEMA_VERSION,
            }) if found == future_ver
        ));
    }
}

#[test]
fn snapshot_freshness_enforces_strict_under_60s_window_and_handles_backward_clock_skew() {
    assert_eq!(SNAPSHOT_MAX_AGE_SECS, TICK_SECS as u64);
    assert_eq!(SNAPSHOT_MAX_AGE_SECS, 60);

    let t0 = Timestamp::from_unix_secs(1_745_000_000);
    let snap = Snapshot::new("◈", "1/8 planted · 1 blooming", Season::Spring, "frame", t0);

    // Backward clock skew (`now < updated_at`) stays fresh so `niwa prompt` does not thrash simulation.
    assert!(snap.is_fresh(
        Timestamp::from_unix_secs(1_744_999_985),
        SNAPSHOT_MAX_AGE_SECS
    ));

    // Exact timestamp (`age == 0 s`) and `age == 59 s` are fresh (`< 60 s`).
    assert!(snap.is_fresh(t0, SNAPSHOT_MAX_AGE_SECS));
    assert!(snap.is_fresh(
        Timestamp::from_unix_secs(1_745_000_059),
        SNAPSHOT_MAX_AGE_SECS
    ));

    // `age == 60 s` (one full garden tick) and older are stale (`>= 60 s`).
    assert!(!snap.is_fresh(
        Timestamp::from_unix_secs(1_745_000_060),
        SNAPSHOT_MAX_AGE_SECS
    ));
    assert!(!snap.is_fresh(
        Timestamp::from_unix_secs(1_745_003_600),
        SNAPSHOT_MAX_AGE_SECS
    ));
}

#[test]
fn prompt_style_display_and_from_str_round_trip() {
    for (raw, expected) in [
        ("glyph", PromptStyle::Glyph),
        ("emoji", PromptStyle::Emoji),
        ("none", PromptStyle::None),
        ("GLYPH", PromptStyle::Glyph),
        (" Emoji ", PromptStyle::Emoji),
    ] {
        let parsed = PromptStyle::from_str(raw).expect("valid prompt style");
        assert_eq!(parsed, expected);
        assert_eq!(parsed.to_string(), expected.as_str());
    }

    assert!(PromptStyle::from_str("sparkles").is_err());
    assert!(PromptStyle::from_str("").is_err());
}
