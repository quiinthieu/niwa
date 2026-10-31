use std::collections::BTreeMap;
use std::path::Path;
use tg_core::balance::Balance;
use tg_core::clock::Timestamp;
use tg_core::drops::DropTable;
use tg_core::model::Stage;
use tg_core::rng::rng_from_seed;
use tg_core::season::Season;
use tg_core::sim::{Garden, TickInput};
use tg_core::species::{Rarity, SpeciesCatalog, EMBEDDED_SPECIES_FILES};

#[test]
fn default_catalog_covers_every_rarity_in_every_season_and_matches_disk_files() {
    let catalog = SpeciesCatalog::load_default();
    assert!(catalog.len() >= 16);

    for season in Season::ALL {
        for rarity in Rarity::ALL {
            let matching = catalog
                .iter()
                .filter(|sp| sp.rarity == rarity && sp.prefers_season(season))
                .count();
            assert!(
                matching >= 1,
                "expected at least one {rarity:?} species preferring {season:?}, found 0"
            );
        }
    }

    // Ensure EMBEDDED_SPECIES_FILES stays 1:1 with data/species/*.toml on disk.
    let species_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .join("data")
        .join("species");

    let mut disk_files: BTreeMap<String, String> = BTreeMap::new();
    for entry in std::fs::read_dir(&species_dir).expect("read data/species directory") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("toml") {
            let file_name = path.file_name().unwrap().to_str().unwrap();
            let rel = format!("data/species/{file_name}");
            let contents = std::fs::read_to_string(&path).expect("read species toml");
            disk_files.insert(rel, contents);
        }
    }

    let embedded_map: BTreeMap<String, String> = EMBEDDED_SPECIES_FILES
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect();

    assert_eq!(
        embedded_map, disk_files,
        "EMBEDDED_SPECIES_FILES must match data/species/*.toml on disk"
    );
}

#[test]
fn bad_species_files_report_clear_error_with_file_and_line() {
    // 1. Malformed TOML syntax on line 3.
    let syntax_bad = "id = \"fern\"\ndisplay_name = \"Fern\"\nrarity = [unclosed\n";
    let err = SpeciesCatalog::parse_species_toml("data/species/bad_syntax.toml", syntax_bad)
        .expect_err("syntax error must fail");
    assert_eq!(err.file(), "data/species/bad_syntax.toml");
    assert_eq!(err.line(), 3);
    assert!(err.to_string().contains("data/species/bad_syntax.toml:3:"));

    // 2. Invalid rarity enum on line 3.
    let bad_rarity = concat!(
        "id = \"fern\"\n",
        "display_name = \"Fern\"\n",
        "rarity = \"legendary\"\n",
        "gp_per_stage = [10, 20, 30, 40]\n",
        "preferred_seasons = [\"spring\"]\n",
        "color_hint = \"green\"\n",
        "almanac_flavour = \"Cozy fern.\"\n",
        "[sprites.unicode]\nseed = \".\"\nsprout = \",\"\nyoung = \"|\"\nmature = \"+\"\nblooming = \"*\"\n",
        "[sprites.ascii]\nseed = \".\"\nsprout = \",\"\nyoung = \"|\"\nmature = \"+\"\nblooming = \"*\"\n",
    );
    let err = SpeciesCatalog::parse_species_toml("data/species/bad_rarity.toml", bad_rarity)
        .expect_err("unknown rarity must fail");
    assert_eq!(err.file(), "data/species/bad_rarity.toml");
    assert_eq!(err.line(), 3);
    assert!(err.to_string().contains("data/species/bad_rarity.toml:3:"));

    // 3. Zero GP threshold on line 4.
    let zero_gp = concat!(
        "id = \"fern\"\n",
        "display_name = \"Fern\"\n",
        "rarity = \"common\"\n",
        "gp_per_stage = [10, 0, 30, 40]\n",
        "preferred_seasons = [\"spring\"]\n",
        "color_hint = \"green\"\n",
        "almanac_flavour = \"Cozy fern.\"\n",
        "[sprites.unicode]\nseed = \".\"\nsprout = \",\"\nyoung = \"|\"\nmature = \"+\"\nblooming = \"*\"\n",
        "[sprites.ascii]\nseed = \".\"\nsprout = \",\"\nyoung = \"|\"\nmature = \"+\"\nblooming = \"*\"\n",
    );
    let err = SpeciesCatalog::parse_species_toml("data/species/zero_gp.toml", zero_gp)
        .expect_err("zero GP threshold must fail");
    assert_eq!(err.file(), "data/species/zero_gp.toml");
    assert_eq!(err.line(), 4);
    assert!(err.to_string().contains("data/species/zero_gp.toml:4:"));

    // 4. Empty preferred_seasons on line 5.
    let empty_seasons = concat!(
        "id = \"fern\"\n",
        "display_name = \"Fern\"\n",
        "rarity = \"common\"\n",
        "gp_per_stage = [10, 20, 30, 40]\n",
        "preferred_seasons = []\n",
        "color_hint = \"green\"\n",
        "almanac_flavour = \"Cozy fern.\"\n",
        "[sprites.unicode]\nseed = \".\"\nsprout = \",\"\nyoung = \"|\"\nmature = \"+\"\nblooming = \"*\"\n",
        "[sprites.ascii]\nseed = \".\"\nsprout = \",\"\nyoung = \"|\"\nmature = \"+\"\nblooming = \"*\"\n",
    );
    let err = SpeciesCatalog::parse_species_toml("data/species/no_seasons.toml", empty_seasons)
        .expect_err("empty preferred_seasons must fail");
    assert_eq!(err.file(), "data/species/no_seasons.toml");
    assert_eq!(err.line(), 5);

    // 5. Non-7-bit-ASCII character in sprites.ascii on line 19.
    let non_ascii = concat!(
        "id = \"fern\"\n",                    // 1
        "display_name = \"Fern\"\n",          // 2
        "rarity = \"common\"\n",              // 3
        "gp_per_stage = [10, 20, 30, 40]\n",  // 4
        "preferred_seasons = [\"spring\"]\n", // 5
        "color_hint = \"green\"\n",           // 6
        "almanac_flavour = \"Cozy fern.\"\n", // 7
        "[sprites.unicode]\n",                // 8
        "seed = \".\"\n",                     // 9
        "sprout = \",\"\n",                   // 10
        "young = \"|\"\n",                    // 11
        "mature = \"+\"\n",                   // 12
        "blooming = \"░\"\n",                 // 13
        "[sprites.ascii]\n",                  // 14
        "seed = \".\"\n",                     // 15
        "sprout = \",\"\n",                   // 16
        "young = \"|\"\n",                    // 17
        "mature = \"+\"\n",                   // 18
        "blooming = \"░\"\n",                 // 19 -> non-ASCII!
    );
    let err = SpeciesCatalog::parse_species_toml("data/species/non_ascii.toml", non_ascii)
        .expect_err("non-ASCII glyph in sprites.ascii must fail");
    assert_eq!(err.file(), "data/species/non_ascii.toml");
    assert_eq!(err.line(), 19);
    assert!(err.to_string().contains("data/species/non_ascii.toml:19:"));

    // 6. Wide emoji in sprites.unicode on line 13.
    let emoji_sprite = concat!(
        "id = \"fern\"\n",                    // 1
        "display_name = \"Fern\"\n",          // 2
        "rarity = \"common\"\n",              // 3
        "gp_per_stage = [10, 20, 30, 40]\n",  // 4
        "preferred_seasons = [\"spring\"]\n", // 5
        "color_hint = \"green\"\n",           // 6
        "almanac_flavour = \"Cozy fern.\"\n", // 7
        "[sprites.unicode]\n",                // 8
        "seed = \".\"\n",                     // 9
        "sprout = \",\"\n",                   // 10
        "young = \"|\"\n",                    // 11
        "mature = \"+\"\n",                   // 12
        "blooming = \"🌸\"\n",                // 13 -> wide emoji forbidden by PLAN §4.3!
        "[sprites.ascii]\n",                  // 14
        "seed = \".\"\n",                     // 15
        "sprout = \",\"\n",                   // 16
        "young = \"|\"\n",                    // 17
        "mature = \"+\"\n",                   // 18
        "blooming = \"*\"\n",                 // 19
    );
    let err = SpeciesCatalog::parse_species_toml("data/species/emoji.toml", emoji_sprite)
        .expect_err("emoji in sprites.unicode must fail");
    assert_eq!(err.file(), "data/species/emoji.toml");
    assert_eq!(err.line(), 13);

    // 7. Duplicate species id across two files reports the second file and id line.
    let file_a = concat!(
        "id = \"fern\"\n",
        "display_name = \"Fern A\"\n",
        "rarity = \"common\"\n",
        "gp_per_stage = [10, 20, 30, 40]\n",
        "preferred_seasons = [\"spring\"]\n",
        "color_hint = \"green\"\n",
        "almanac_flavour = \"First fern.\"\n",
        "[sprites.unicode]\nseed = \".\"\nsprout = \",\"\nyoung = \"|\"\nmature = \"+\"\nblooming = \"*\"\n",
        "[sprites.ascii]\nseed = \".\"\nsprout = \",\"\nyoung = \"|\"\nmature = \"+\"\nblooming = \"*\"\n",
    );
    let file_b = concat!(
        "# comment line 1\n",
        "id = \"fern\"\n", // line 2
        "display_name = \"Fern B\"\n",
        "rarity = \"common\"\n",
        "gp_per_stage = [10, 20, 30, 40]\n",
        "preferred_seasons = [\"spring\"]\n",
        "color_hint = \"green\"\n",
        "almanac_flavour = \"Duplicate fern.\"\n",
        "[sprites.unicode]\nseed = \".\"\nsprout = \",\"\nyoung = \"|\"\nmature = \"+\"\nblooming = \"*\"\n",
        "[sprites.ascii]\nseed = \".\"\nsprout = \",\"\nyoung = \"|\"\nmature = \"+\"\nblooming = \"*\"\n",
    );
    let err = SpeciesCatalog::from_named_sources([
        ("data/species/fern_a.toml", file_a),
        ("data/species/fern_b.toml", file_b),
    ])
    .expect_err("duplicate id must fail");
    assert_eq!(err.file(), "data/species/fern_b.toml");
    assert_eq!(err.line(), 2);
}

#[test]
fn seasonal_specialist_rests_as_seed_out_of_season_and_sprouts_in_season() {
    let balance = Balance::load_default();
    let catalog = SpeciesCatalog::load_default();
    let sakura = catalog.get("sakura").expect("sakura in catalog");
    assert!(sakura.seasonal_sprout_only);

    let mut garden = Garden::new(42, 2, Timestamp::from_unix_secs(1_700_000_000));
    garden.plant_seed(0, "sakura").unwrap();
    garden.plant_seed(1, "clover").unwrap();

    // In Winter, sakura (seasonal_sprout_only = true, spring) rests at Seed with 0 GP,
    // while clover (year-round sprouting, off-season in winter) still grows at off-season rate.
    for _ in 0..500 {
        garden.tick_with_catalog(TickInput::active(), Season::Winter, &balance, &catalog);
    }
    assert_eq!(garden.plots()[0].plant().unwrap().stage, Stage::Seed);
    assert_eq!(garden.plots()[0].plant().unwrap().total_gp, 0);
    assert!(garden.plots()[1].plant().unwrap().total_gp > 0);

    // When Spring arrives, sakura begins accumulating GP and sprouts past its 80_000 GP Seed threshold!
    for _ in 0..400 {
        garden.tick_with_catalog(TickInput::active(), Season::Spring, &balance, &catalog);
    }
    assert!(garden.plots()[0].plant().unwrap().stage >= Stage::Sprout);
    assert!(garden.plots()[0].plant().unwrap().total_gp >= 80_000);
}

#[test]
fn drop_table_sampling_over_fixed_seed_matches_golden_and_respects_rarity_and_season() {
    let balance = Balance::load_default();
    let catalog = SpeciesCatalog::load_default();
    let table = DropTable::new(balance.drops);
    let mut rng = rng_from_seed(0x4E49_5741_2026_0004);

    let mut lines = Vec::new();
    for season in Season::ALL {
        let picks: Vec<String> = (0..12)
            .map(|_| {
                let sp = table
                    .sample_harvest(&catalog, season, &mut rng)
                    .expect("non-empty drop pool");
                format!("{}({})", sp.id, sp.rarity.label())
            })
            .collect();
        lines.push(format!("{}: {}", season.as_str(), picks.join(", ")));
    }

    // Same-rarity compost draws always preserve the requested rarity tier.
    for rarity in Rarity::ALL {
        for _ in 0..25 {
            let sp = table
                .sample_same_rarity(&catalog, rarity, Season::Autumn, &mut rng)
                .expect("non-empty rarity pool");
            assert_eq!(sp.rarity, rarity);
        }
    }

    insta::assert_snapshot!("drop_table_fixed_seed_golden", lines.join("\n"));

    // Empirical check over 4_000 draws in Spring: Common > Uncommon > Rare > Mythic > 0,
    // and in-season Uncommon (primrose) drops more often than an off-season Uncommon (snowdrop).
    let mut tier_counts = [0_usize; 4];
    let mut primrose_count = 0_usize;
    let mut snowdrop_count = 0_usize;
    for _ in 0..4_000 {
        let sp = table
            .sample_harvest(&catalog, Season::Spring, &mut rng)
            .unwrap();
        let idx = match sp.rarity {
            Rarity::Common => 0,
            Rarity::Uncommon => 1,
            Rarity::Rare => 2,
            Rarity::Mythic => 3,
        };
        tier_counts[idx] += 1;
        if sp.id == "primrose" {
            primrose_count += 1;
        } else if sp.id == "snowdrop" {
            snowdrop_count += 1;
        }
    }
    assert!(tier_counts[0] > tier_counts[1]);
    assert!(tier_counts[1] > tier_counts[2]);
    assert!(tier_counts[2] > tier_counts[3]);
    assert!(tier_counts[3] > 0);
    assert!(primrose_count > snowdrop_count);
}
