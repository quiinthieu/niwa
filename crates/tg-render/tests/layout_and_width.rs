use proptest::prelude::*;
use tg_core::clock::Timestamp;
use tg_core::model::{Plant, Plot, Stage};
use tg_core::season::Season;
use tg_core::sim::Garden;
use tg_core::species::{SpeciesCatalog, SpriteSet};
use tg_render::color::ColorTier;
use tg_render::layout::{
    render_compact_frame, render_garden, LayoutKind, RenderOptions, Viewport, MIN_GRID_HEIGHT,
    MIN_GRID_WIDTH,
};
use tg_render::sprites::SpriteBank;
use tg_render::width::{
    display_width, pad_right_to_width, strip_ansi, truncate_to_width, visible_width,
};

fn mixed_garden(dormant: bool) -> Garden {
    let t0 = Timestamp::from_unix_secs(1_761_800_000);
    let specs: [Option<(&str, Stage, u64, u64)>; 9] = [
        Some(("daisy", Stage::Blooming, 0, 720_000)),
        Some(("sakura", Stage::Mature, 320_000, 1_040_000)),
        Some(("clover", Stage::Young, 95_000, 241_000)),
        None,
        Some(("moss", Stage::Sprout, 45_000, 80_000)),
        Some(("lavender", Stage::Seed, 12_000, 12_000)),
        Some(("moonflower", Stage::Blooming, 0, 1_387_000)),
        None,
        Some(("red_maple", Stage::Mature, 245_000, 812_000)),
    ];

    let plots: Vec<Plot> = specs
        .into_iter()
        .map(|entry| match entry {
            None => Plot::empty(),
            Some((id, stage, stage_gp, total_gp)) => Plot::with_plant(Plant {
                species_id: id.to_string(),
                stage,
                stage_gp,
                total_gp,
                planted_at: t0,
                planted_tick: 0,
                bloomed_at: (stage == Stage::Blooming).then_some(t0.add_secs(3_600)),
                bloom_seed: (stage == Stage::Blooming).then_some(99),
            }),
        })
        .collect();

    Garden::from_parts(
        42,
        plots,
        t0,
        t0.add_secs(86_400),
        1_440,
        if dormant { 10_080 } else { 0 },
        dormant,
    )
}

#[test]
fn grid_switches_to_compact_list_below_40x12_threshold() {
    let catalog = SpeciesCatalog::load_default();
    let sprites = SpriteBank::load_default();
    let garden = mixed_garden(false);
    let opts = RenderOptions {
        sprites: SpriteSet::Unicode,
        color: ColorTier::NoColor,
        season: Season::Spring,
    };

    let at_min = render_garden(
        &garden,
        &catalog,
        &sprites,
        Viewport::new(MIN_GRID_WIDTH, MIN_GRID_HEIGHT),
        opts,
    );
    assert_eq!(at_min.layout(), LayoutKind::Grid);

    for (w, h) in [(39, 12), (40, 11), (32, 10), (20, 6), (8, 3)] {
        let frame = render_garden(&garden, &catalog, &sprites, Viewport::new(w, h), opts);
        assert_eq!(
            frame.layout(),
            LayoutKind::CompactList,
            "viewport {w}x{h} below 40x12 must fall back to CompactList"
        );
        let text = frame.to_ansi_string();
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines.len() <= usize::from(h));
        for line in lines {
            assert!(
                visible_width(line) <= usize::from(w),
                "line {line:?} exceeded width {w}"
            );
        }
    }

    // Zero dimensions never panic and return an empty string.
    let zero_w = render_garden(&garden, &catalog, &sprites, Viewport::new(0, 12), opts);
    assert!(zero_w.lines().is_empty());
    assert_eq!(zero_w.to_ansi_string(), "");
}

#[test]
fn color_tiers_preserve_exact_plain_geometry_and_seasons_change_palette() {
    let catalog = SpeciesCatalog::load_default();
    let sprites = SpriteBank::load_default();
    let garden = mixed_garden(true);

    for viewport in [
        Viewport::new(32, 10),
        Viewport::new(40, 12),
        Viewport::new(80, 24),
        Viewport::new(120, 40),
    ] {
        for set in SpriteSet::ALL {
            let plain = render_garden(
                &garden,
                &catalog,
                &sprites,
                viewport,
                RenderOptions {
                    sprites: set,
                    color: ColorTier::NoColor,
                    season: Season::Autumn,
                },
            )
            .to_ansi_string();
            assert!(
                !plain.contains('\x1b'),
                "NoColor output must contain zero ANSI escape codes"
            );
            // Dormancy must be communicated explicitly in text (no color-only state indicators).
            if viewport.width >= 32 {
                assert!(
                    plain.contains("Dormant") || plain.contains("REST"),
                    "dormant state must be visible in plain NoColor text at {}x{}",
                    viewport.width,
                    viewport.height
                );
            }

            for tier in [ColorTier::Ansi16, ColorTier::Ansi256, ColorTier::Truecolor] {
                let colored = render_garden(
                    &garden,
                    &catalog,
                    &sprites,
                    viewport,
                    RenderOptions {
                        sprites: set,
                        color: tier,
                        season: Season::Autumn,
                    },
                )
                .to_ansi_string();
                assert!(
                    colored.contains("\x1b["),
                    "colored tier {tier:?} must emit ANSI SGR codes"
                );
                assert_eq!(
                    strip_ansi(&colored),
                    plain,
                    "stripping ANSI from {tier:?} must equal NoColor character-for-character"
                );
            }
        }
    }

    // Verify each season uses its own distinct palette in Truecolor, Ansi256, and Ansi16.
    for tier in [ColorTier::Ansi16, ColorTier::Ansi256, ColorTier::Truecolor] {
        let mut by_season = Vec::new();
        for season in Season::ALL {
            let rendered = render_garden(
                &garden,
                &catalog,
                &sprites,
                Viewport::new(80, 24),
                RenderOptions {
                    sprites: SpriteSet::Unicode,
                    color: tier,
                    season,
                },
            )
            .to_ansi_string();
            by_season.push(rendered);
        }
        for i in 0..by_season.len() {
            for j in (i + 1)..by_season.len() {
                assert_ne!(
                    by_season[i], by_season[j],
                    "seasons must render distinct seasonal palettes/frames under {tier:?}"
                );
            }
        }
    }
}

#[test]
fn wide_characters_and_combining_marks_never_overflow_cell_or_viewport_widths() {
    // Odd max_width truncation with width-2 CJK characters must never overshoot by 1 cell.
    assert_eq!(display_width("桜盆栽"), 6);
    let truncated_5 = truncate_to_width("桜盆栽", 5);
    assert!(display_width(&truncated_5) <= 5);
    let padded_5 = pad_right_to_width(&truncated_5, 5);
    assert_eq!(display_width(&padded_5), 5);

    // Combining acute accent (U+0301) has 0 extra column width.
    assert_eq!(display_width("Cafe\u{0301}"), 4);

    // Build a custom catalog whose display names contain wide CJK characters and combining marks.
    let wide_daisy = concat!(
        "id = \"daisy\"\n",
        "display_name = \"桜盆栽と野菊 Café\u{0301} Blossom\"\n",
        "rarity = \"common\"\n",
        "gp_per_stage = [40000, 120000, 220000, 340000]\n",
        "preferred_seasons = [\"spring\"]\n",
        "color_hint = \"gold\"\n",
        "almanac_flavour = \"Wide display name safety check.\"\n",
        "[sprites.unicode]\nseed = \" · \"\nsprout = \" ╷ \"\nyoung = \" │ \"\nmature = \"├│┤\"\nblooming = \"░█░\"\n",
        "[sprites.ascii]\nseed = \" . \"\nsprout = \" , \"\nyoung = \" | \"\nmature = \"-|-\"\nblooming = \"*O*\"\n",
    );
    let catalog = SpeciesCatalog::from_named_sources([("wide_daisy.toml", wide_daisy)])
        .expect("valid custom species with wide display name");
    let sprites = SpriteBank::load_default();
    let t0 = Timestamp::from_unix_secs(1_761_800_000);
    let mut garden = Garden::new(1, 9, t0);
    for idx in 0..9 {
        garden.plant_seed(idx, "daisy").unwrap();
    }

    for (w, h) in [(17, 7), (31, 11), (40, 12), (41, 13), (80, 24), (120, 40)] {
        for tier in ColorTier::ALL {
            let rendered = render_garden(
                &garden,
                &catalog,
                &sprites,
                Viewport::new(w, h),
                RenderOptions {
                    sprites: SpriteSet::Unicode,
                    color: tier,
                    season: Season::Spring,
                },
            )
            .to_ansi_string();
            let lines: Vec<&str> = rendered.lines().collect();
            assert!(lines.len() <= usize::from(h));
            for line in lines {
                assert!(
                    visible_width(line) <= usize::from(w),
                    "wide-char line {line:?} (width {}) overflowed viewport width {w}",
                    visible_width(line)
                );
            }
        }
    }

    let compact = render_compact_frame(
        &garden,
        &catalog,
        &sprites,
        SpriteSet::Unicode,
        Season::Spring,
    );
    assert!(!compact.contains('\x1b'));
    for line in compact.lines() {
        assert!(visible_width(line) <= usize::from(MIN_GRID_WIDTH));
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    #[test]
    fn no_line_ever_overflows_viewport_width_or_height(
        width in 1_u16..=150,
        height in 1_u16..=55,
        plot_count in 1_usize..=12,
        ascii in any::<bool>(),
        tier_idx in 0_usize..4,
        season_idx in 0_usize..4,
        dormant in any::<bool>(),
    ) {
        let catalog = SpeciesCatalog::load_default();
        let sprites = SpriteBank::load_default();
        let t0 = Timestamp::from_unix_secs(1_761_800_000);
        let species_ids = ["daisy", "sakura", "moss", "moonflower", "lotus", "red_maple"];
        let stages = [Stage::Seed, Stage::Sprout, Stage::Young, Stage::Mature, Stage::Blooming];

        let plots: Vec<Plot> = (0..plot_count)
            .map(|idx| {
                if idx % 4 == 3 {
                    Plot::empty()
                } else {
                    let stage = stages[idx % stages.len()];
                    Plot::with_plant(Plant {
                        species_id: species_ids[idx % species_ids.len()].to_string(),
                        stage,
                        stage_gp: 15_000,
                        total_gp: 120_000,
                        planted_at: t0,
                        planted_tick: 0,
                        bloomed_at: (stage == Stage::Blooming).then_some(t0),
                        bloom_seed: (stage == Stage::Blooming).then_some(7),
                    })
                }
            })
            .collect();

        let garden = Garden::from_parts(9, plots, t0, t0, 100, 0, dormant);
        let frame = render_garden(
            &garden,
            &catalog,
            &sprites,
            Viewport::new(width, height),
            RenderOptions {
                sprites: if ascii { SpriteSet::Ascii } else { SpriteSet::Unicode },
                color: ColorTier::ALL[tier_idx],
                season: Season::ALL[season_idx],
            },
        );

        let out = frame.to_ansi_string();
        let lines: Vec<&str> = out.lines().collect();
        prop_assert!(lines.len() <= usize::from(height));
        for line in lines {
            prop_assert!(
                visible_width(line) <= usize::from(width),
                "line width {} exceeded viewport width {} at {}x{}",
                visible_width(line),
                width,
                width,
                height
            );
        }
    }
}
