use tg_core::clock::Timestamp;
use tg_core::model::{Plant, Plot, Stage};
use tg_core::season::Season;
use tg_core::sim::Garden;
use tg_core::species::{is_forbidden_emoji_char, SpeciesCatalog, SpriteSet};
use tg_render::color::ColorTier;
use tg_render::layout::{render_garden, render_garden_with_phase, RenderOptions, Viewport};
use tg_render::sprites::SpriteBank;
use tg_render::width::visible_width;

fn ambience_garden(dormant: bool) -> Garden {
    let t0 = Timestamp::from_unix_secs(1_761_800_000);
    let specs: [Option<(&str, Stage, u64, u64)>; 9] = [
        Some(("sakura", Stage::Blooming, 0, 1_360_000)),
        Some(("lavender", Stage::Mature, 260_000, 708_000)),
        Some(("red_maple", Stage::Blooming, 0, 1_057_000)),
        Some(("frostbell", Stage::Blooming, 0, 1_402_000)),
        None,
        Some(("chamomile", Stage::Young, 140_000, 293_000)),
        Some(("lotus", Stage::Mature, 300_000, 885_000)),
        Some(("snowdrop", Stage::Sprout, 90_000, 134_000)),
        None,
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
                bloomed_at: (stage == Stage::Blooming).then_some(t0.add_secs(7_200)),
                bloom_seed: (stage == Stage::Blooming).then_some(2_026),
            }),
        })
        .collect();

    Garden::from_parts(
        2_026,
        plots,
        t0,
        t0.add_secs(172_800),
        2_880,
        if dormant { 10_080 } else { 0 },
        dormant,
    )
}

#[test]
fn seasonal_ambience_renders_distinct_motifs_and_gentle_phases_across_80x24_and_120x40() {
    let catalog = SpeciesCatalog::load_default();
    let sprites = SpriteBank::load_default();
    let active_garden = ambience_garden(false);
    let dormant_garden = ambience_garden(true);

    let season_keywords = [
        (Season::Spring, "petals"),
        (Season::Summer, "fireflies"),
        (Season::Autumn, "leaves"),
        (Season::Winter, "snow"),
    ];

    for viewport in [Viewport::new(80, 24), Viewport::new(120, 40)] {
        for set in SpriteSet::ALL {
            let mut phase0_by_season = Vec::new();

            for (season, expected_keyword) in season_keywords {
                let opts = RenderOptions {
                    sprites: set,
                    color: ColorTier::NoColor,
                    season,
                };

                let base = render_garden(&active_garden, &catalog, &sprites, viewport, opts);
                let phase0 =
                    render_garden_with_phase(&active_garden, &catalog, &sprites, viewport, opts, 0);
                assert_eq!(
                    base, phase0,
                    "render_garden must match render_garden_with_phase(..., 0)"
                );

                let text0 = phase0.to_plain_string();
                assert!(
                    text0.contains(expected_keyword),
                    "expected {season:?} ({set:?}) at {}x{} to feature '{expected_keyword}', got:\n{text0}",
                    viewport.width,
                    viewport.height
                );

                let mut phase_frames = Vec::new();
                for phase in 0..4_u8 {
                    let frame = render_garden_with_phase(
                        &active_garden,
                        &catalog,
                        &sprites,
                        viewport,
                        opts,
                        phase,
                    );
                    let plain = frame.to_plain_string();
                    let lines: Vec<&str> = plain.lines().collect();
                    assert_eq!(lines.len(), usize::from(viewport.height));
                    for line in &lines {
                        assert_eq!(
                            visible_width(line),
                            usize::from(viewport.width),
                            "grid line must match exact viewport width {}",
                            viewport.width
                        );
                    }

                    match set {
                        SpriteSet::Unicode => {
                            assert!(
                                !plain.chars().any(is_forbidden_emoji_char),
                                "unicode seasonal ambience must remain emoji-free (PLAN §4.3)"
                            );
                        }
                        SpriteSet::Ascii => {
                            assert!(
                                plain
                                    .bytes()
                                    .all(|b| b == b'\n' || (0x20..=0x7E).contains(&b)),
                                "ascii seasonal ambience must use 7-bit printable ASCII only"
                            );
                        }
                    }

                    phase_frames.push(plain);
                }

                // Active gardens gently shift ambient particles across the 4-frame cycle.
                assert!(
                    phase_frames.windows(2).any(|pair| pair[0] != pair[1]),
                    "expected ambient phase drift for active {season:?}/{set:?} at {}x{}",
                    viewport.width,
                    viewport.height
                );

                // Phase wraps cleanly modulo 4 so animation is bounded and deterministic.
                let phase4 =
                    render_garden_with_phase(&active_garden, &catalog, &sprites, viewport, opts, 4)
                        .to_plain_string();
                assert_eq!(phase_frames[0], phase4);

                // Dormant gardens keep ambient particles settled at rest across phases.
                let dormant0 = render_garden_with_phase(
                    &dormant_garden,
                    &catalog,
                    &sprites,
                    viewport,
                    opts,
                    0,
                )
                .to_plain_string();
                for phase in 1..4_u8 {
                    let dormant_p = render_garden_with_phase(
                        &dormant_garden,
                        &catalog,
                        &sprites,
                        viewport,
                        opts,
                        phase,
                    )
                    .to_plain_string();
                    assert_eq!(
                        dormant0, dormant_p,
                        "dormant garden ambience must rest still across phases"
                    );
                }

                phase0_by_season.push(text0);
            }

            // All 4 seasons must look distinct even in plain NoColor text.
            for i in 0..phase0_by_season.len() {
                for j in (i + 1)..phase0_by_season.len() {
                    assert_ne!(phase0_by_season[i], phase0_by_season[j]);
                }
            }
        }
    }
}

#[test]
fn golden_seasonal_ambience_across_all_four_seasons() {
    let catalog = SpeciesCatalog::load_default();
    let sprites = SpriteBank::load_default();
    let garden = ambience_garden(false);

    let mut combined_80x24 = Vec::new();
    for season in Season::ALL {
        for set in SpriteSet::ALL {
            let rendered = render_garden_with_phase(
                &garden,
                &catalog,
                &sprites,
                Viewport::new(80, 24),
                RenderOptions {
                    sprites: set,
                    color: ColorTier::NoColor,
                    season,
                },
                1,
            )
            .to_plain_string();
            combined_80x24.push(format!(
                "=== 80x24 {} {} (phase 1) ===\n{}",
                season.as_str(),
                set.as_str(),
                rendered
            ));
        }
    }
    insta::assert_snapshot!(
        "seasonal_ambience_80x24_all_seasons",
        combined_80x24.join("\n\n")
    );
}
