use std::collections::BTreeMap;
use std::path::Path;
use tg_core::balance::Balance;
use tg_core::clock::Timestamp;
use tg_core::model::Stage;
use tg_core::sim::{Garden, TickInput};
use tg_core::species::{is_forbidden_emoji_char, SpeciesCatalog, SpriteSet};
use tg_render::color::{detect_color_tier, ColorEnv, ColorPreference, ColorTier};
use tg_render::glyph::{compact_glyph, most_advanced_stage, prompt_emoji_glyph, stage_glyph};
use tg_render::sprites::{
    SpriteBank, EMBEDDED_ASCII_SPRITES, EMBEDDED_UNICODE_SPRITES, SPRITE_ART_HEIGHT,
    SPRITE_ART_WIDTH,
};
use tg_render::width::display_width;

#[test]
fn sprite_bank_covers_every_species_and_stage_in_unicode_and_ascii_and_matches_disk() {
    let catalog = SpeciesCatalog::load_default();
    let bank = SpriteBank::load_default();

    for species in catalog.iter() {
        for stage in Stage::ALL {
            let unicode_rows = bank.art_for(SpriteSet::Unicode, &species.id, stage);
            assert_eq!(unicode_rows.len(), SPRITE_ART_HEIGHT);
            for row in unicode_rows {
                assert_eq!(
                    display_width(row),
                    SPRITE_ART_WIDTH,
                    "unicode sprite row for {}:{stage:?} must be {} cols wide, got {:?}",
                    species.id,
                    SPRITE_ART_WIDTH,
                    row
                );
                assert!(
                    !row.chars().any(is_forbidden_emoji_char),
                    "unicode sprite row for {}:{stage:?} contains forbidden emoji: {:?}",
                    species.id,
                    row
                );
            }

            let ascii_rows = bank.art_for(SpriteSet::Ascii, &species.id, stage);
            assert_eq!(ascii_rows.len(), SPRITE_ART_HEIGHT);
            for row in ascii_rows {
                assert_eq!(
                    display_width(row),
                    SPRITE_ART_WIDTH,
                    "ascii sprite row for {}:{stage:?} must be {} cols wide, got {:?}",
                    species.id,
                    SPRITE_ART_WIDTH,
                    row
                );
                assert!(
                    row.bytes().all(|b| (0x20..=0x7E).contains(&b)),
                    "ascii sprite row for {}:{stage:?} must be 7-bit printable ASCII, got {:?}",
                    species.id,
                    row
                );
            }
        }
    }

    // Verify embedded sprite maps stay 1:1 with data/sprites/{unicode,ascii}/*.toml on disk.
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");

    for (subdir, embedded) in [
        ("unicode", EMBEDDED_UNICODE_SPRITES),
        ("ascii", EMBEDDED_ASCII_SPRITES),
    ] {
        let dir = workspace_root.join("data").join("sprites").join(subdir);
        let mut disk_map = BTreeMap::new();
        for entry in std::fs::read_dir(&dir).expect("read data/sprites subdir") {
            let entry = entry.expect("dir entry");
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("toml") {
                let name = path.file_name().unwrap().to_str().unwrap();
                let rel = format!("data/sprites/{subdir}/{name}");
                let contents = std::fs::read_to_string(&path).expect("read sprite toml");
                disk_map.insert(rel, contents);
            }
        }
        let embedded_map: BTreeMap<String, String> = embedded
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        assert_eq!(
            embedded_map, disk_map,
            "embedded {subdir} sprites must match data/sprites/{subdir}/*.toml on disk"
        );
    }
}

#[test]
fn invalid_sprite_files_report_clear_file_line_and_reason() {
    // Wrong row width in Unicode sprite.
    let bad_width = concat!(
        "species_id = \"daisy\"\n",
        "seed = [\"       \", \"   ·   \", \"too_wide_row\"]\n",
        "sprout = [\"       \", \"   ╷   \", \"  ─┴─  \"]\n",
        "young = [\"   ╷   \", \"  ├│┤  \", \"  ─┴─  \"]\n",
        "mature = [\"   ◎   \", \"  ├│┤  \", \"  ─┴─  \"]\n",
        "blooming = [\" ░▓█▓░ \", \"  ├│┤  \", \"  ─┴─  \"]\n",
    );
    let err = SpriteBank::parse_sprite_toml(
        "data/sprites/unicode/bad_width.toml",
        SpriteSet::Unicode,
        bad_width,
    )
    .expect_err("overwide row must fail validation");
    assert_eq!(err.file(), "data/sprites/unicode/bad_width.toml");
    assert_eq!(err.line(), 2);

    // Forbidden emoji in Unicode sprite.
    let bad_emoji = concat!(
        "species_id = \"daisy\"\n",
        "seed = [\"       \", \"       \", \"  ·•·  \"]\n",
        "sprout = [\"       \", \"   🌱  \", \"  ─┴─  \"]\n",
        "young = [\"   ╷   \", \"  ├│┤  \", \"  ─┴─  \"]\n",
        "mature = [\"   ◎   \", \"  ├│┤  \", \"  ─┴─  \"]\n",
        "blooming = [\" ░▓█▓░ \", \"  ├│┤  \", \"  ─┴─  \"]\n",
    );
    let err = SpriteBank::parse_sprite_toml(
        "data/sprites/unicode/bad_emoji.toml",
        SpriteSet::Unicode,
        bad_emoji,
    )
    .expect_err("emoji in sprite must fail per PLAN §4.3");
    assert_eq!(err.line(), 3);
    assert!(err.message().contains("emoji"));

    // Non-ASCII byte in ASCII sprite.
    let bad_ascii = concat!(
        "species_id = \"daisy\"\n",
        "seed = [\"       \", \"       \", \"  ···  \"]\n",
        "sprout = [\"       \", \"   ,   \", \"  -+-  \"]\n",
        "young = [\"   |   \", \"  -|-  \", \"  -+-  \"]\n",
        "mature = [\"   o   \", \"  -|-  \", \"  -+-  \"]\n",
        "blooming = [\" .*O*. \", \"  -|-  \", \"  -+-  \"]\n",
    );
    let err = SpriteBank::parse_sprite_toml(
        "data/sprites/ascii/bad_ascii.toml",
        SpriteSet::Ascii,
        bad_ascii,
    )
    .expect_err("non-7-bit char in ascii sprite must fail");
    assert_eq!(err.line(), 2);
}

#[test]
fn compact_glyph_is_one_cell_and_tracks_most_advanced_plant() {
    let t0 = Timestamp::from_unix_secs(1_761_800_000);
    let balance = Balance::load_default();
    let catalog = SpeciesCatalog::load_default();
    let mut garden = Garden::new(42, balance.default_plots, t0);

    // Empty garden.
    assert_eq!(most_advanced_stage(&garden), None);
    assert_eq!(compact_glyph(&garden, SpriteSet::Unicode), "◌");
    assert_eq!(compact_glyph(&garden, SpriteSet::Ascii), "_");

    // Verify all 6 stage glyphs (None + 5 stages) are distinct, 1 cell wide, and emoji-free.
    let all_opts = [
        None,
        Some(Stage::Seed),
        Some(Stage::Sprout),
        Some(Stage::Young),
        Some(Stage::Mature),
        Some(Stage::Blooming),
    ];
    for set in SpriteSet::ALL {
        let mut seen = Vec::new();
        for opt in all_opts {
            let g = stage_glyph(opt, set);
            assert_eq!(
                display_width(g),
                1,
                "compact glyph {g:?} for {opt:?}/{set:?} must occupy exactly 1 cell"
            );
            assert!(!g.chars().any(is_forbidden_emoji_char));
            if set == SpriteSet::Ascii {
                assert!(g.is_ascii());
            }
            assert!(
                !seen.contains(&g),
                "compact glyphs in {set:?} must be unique across stages"
            );
            seen.push(g);
        }
    }

    // Plant daisy in plot 0 and moss in plot 4; advance moss further via ticks and plant a fresh seed in plot 8.
    garden.plant_seed(0, "sakura").expect("plant sakura");
    garden.plant_seed(4, "moss").expect("plant moss");
    assert_eq!(most_advanced_stage(&garden), Some(Stage::Seed));
    assert_eq!(compact_glyph(&garden, SpriteSet::Unicode), "·");
    assert_eq!(compact_glyph(&garden, SpriteSet::Ascii), ".");

    // Advance enough active ticks in Winter so moss blooms while sakura (spring-only sprout) stays at Seed.
    // 1_765_800_000 is mid-December UTC (Winter in northern hemisphere).
    let winter_t0 = Timestamp::from_unix_secs(1_765_800_000);
    let mut winter_garden = Garden::new(7, 4, winter_t0);
    winter_garden.plant_seed(0, "sakura").unwrap();
    winter_garden.plant_seed(2, "moss").unwrap();
    for _ in 0..3_500 {
        winter_garden.tick_with_catalog(
            TickInput::active(),
            tg_core::season::Season::Winter,
            &balance,
            &catalog,
        );
    }
    assert_eq!(
        winter_garden.plots()[0].plant().unwrap().stage,
        Stage::Seed,
        "sakura remains Seed in winter"
    );
    assert_eq!(
        winter_garden.plots()[2].plant().unwrap().stage,
        Stage::Blooming,
        "moss reaches Blooming in winter"
    );
    assert_eq!(most_advanced_stage(&winter_garden), Some(Stage::Blooming));
    assert_eq!(compact_glyph(&winter_garden, SpriteSet::Unicode), "◈");
    assert_eq!(compact_glyph(&winter_garden, SpriteSet::Ascii), "*");

    // Prompt emoji helper (strictly isolated for `prompt --style emoji`).
    assert!(!prompt_emoji_glyph(Some(Stage::Blooming)).is_empty());
}

#[test]
fn color_tier_detection_honours_no_color_flag_env_and_terminal_capabilities() {
    // --no-color CLI flag overrides explicit Truecolor preference.
    let env_flag = ColorEnv {
        no_color_flag: true,
        no_color_env: None,
        colorterm: Some("truecolor"),
        term: Some("xterm-256color"),
    };
    assert_eq!(
        detect_color_tier(ColorPreference::Truecolor, &env_flag),
        ColorTier::NoColor
    );

    // Non-empty NO_COLOR env overrides explicit Truecolor preference (https://no-color.org).
    let env_no_color = ColorEnv {
        no_color_flag: false,
        no_color_env: Some("1"),
        colorterm: Some("truecolor"),
        term: Some("xterm-256color"),
    };
    assert_eq!(
        detect_color_tier(ColorPreference::Truecolor, &env_no_color),
        ColorTier::NoColor
    );

    // Empty NO_COLOR="" does NOT disable color per https://no-color.org.
    let env_empty_no_color = ColorEnv {
        no_color_flag: false,
        no_color_env: Some(""),
        colorterm: Some("24bit"),
        term: Some("xterm-256color"),
    };
    assert_eq!(
        detect_color_tier(ColorPreference::Auto, &env_empty_no_color),
        ColorTier::Truecolor
    );

    // 256-color detection from TERM.
    let env_256 = ColorEnv {
        no_color_flag: false,
        no_color_env: None,
        colorterm: None,
        term: Some("tmux-256color"),
    };
    assert_eq!(
        detect_color_tier(ColorPreference::Auto, &env_256),
        ColorTier::Ansi256
    );

    // 16-color fallback on basic non-dumb TERM.
    let env_16 = ColorEnv {
        no_color_flag: false,
        no_color_env: None,
        colorterm: None,
        term: Some("xterm"),
    };
    assert_eq!(
        detect_color_tier(ColorPreference::Auto, &env_16),
        ColorTier::Ansi16
    );

    // TERM=dumb forces NoColor in Auto mode.
    let env_dumb = ColorEnv {
        no_color_flag: false,
        no_color_env: None,
        colorterm: None,
        term: Some("dumb"),
    };
    assert_eq!(
        detect_color_tier(ColorPreference::Auto, &env_dumb),
        ColorTier::NoColor
    );
}
