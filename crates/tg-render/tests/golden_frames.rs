use tg_core::clock::Timestamp;
use tg_core::model::{Plant, Plot, Stage};
use tg_core::season::Season;
use tg_core::sim::Garden;
use tg_core::species::{SpeciesCatalog, SpriteSet};
use tg_render::color::ColorTier;
use tg_render::layout::{render_garden, LayoutKind, RenderOptions, Viewport};
use tg_render::sprites::SpriteBank;
use tg_render::width::visible_width;

fn showcase_garden() -> Garden {
    let t0 = Timestamp::from_unix_secs(1_761_800_000);
    let specs: [Option<(&str, Stage, u64, u64)>; 9] = [
        Some(("sakura", Stage::Blooming, 0, 1_360_000)),
        Some(("daisy", Stage::Mature, 238_000, 618_000)),
        Some(("lily_of_the_valley", Stage::Young, 160_000, 390_000)),
        Some(("clover", Stage::Sprout, 66_000, 102_000)),
        None,
        Some(("primrose", Stage::Seed, 18_000, 18_000)),
        Some(("red_maple", Stage::Blooming, 0, 1_057_000)),
        Some(("moss", Stage::Mature, 217_000, 557_000)),
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
                bloom_seed: (stage == Stage::Blooming).then_some(1_234),
            }),
        })
        .collect();

    Garden::from_parts(2_026, plots, t0, t0.add_secs(172_800), 2_880, 120, false)
}

fn escape_ansi_for_snapshot(raw: &str) -> String {
    raw.replace('\x1b', "\\e")
}

#[test]
fn golden_frames_across_40x12_80x24_120x40_sprite_sets_and_color_tiers() {
    let catalog = SpeciesCatalog::load_default();
    let sprites = SpriteBank::load_default();
    let garden = showcase_garden();

    let sizes = [
        ("40x12", Viewport::new(40, 12)),
        ("80x24", Viewport::new(80, 24)),
        ("120x40", Viewport::new(120, 40)),
    ];

    for (size_label, viewport) in sizes {
        for sprite_set in SpriteSet::ALL {
            for color_tier in ColorTier::ALL {
                let frame = render_garden(
                    &garden,
                    &catalog,
                    &sprites,
                    viewport,
                    RenderOptions {
                        sprites: sprite_set,
                        color: color_tier,
                        season: Season::Spring,
                    },
                );
                assert_eq!(frame.layout(), LayoutKind::Grid);

                let raw = frame.to_ansi_string();
                let lines: Vec<&str> = raw.lines().collect();
                assert_eq!(
                    lines.len(),
                    usize::from(viewport.height),
                    "grid frame at {size_label} must fill {} rows",
                    viewport.height
                );
                for (row_idx, line) in lines.iter().enumerate() {
                    let width = visible_width(line);
                    assert!(
                        width <= usize::from(viewport.width),
                        "row {row_idx} at {size_label} ({:?}/{:?}) overflowed width {}: got {width}",
                        sprite_set,
                        color_tier,
                        viewport.width
                    );
                }

                let snap_name = format!(
                    "frame_{}_{}_{}",
                    size_label,
                    sprite_set.as_str(),
                    color_tier.as_str()
                );
                insta::assert_snapshot!(snap_name, escape_ansi_for_snapshot(&raw));
            }
        }
    }
}

#[test]
fn golden_compact_list_fallback_frames() {
    let catalog = SpeciesCatalog::load_default();
    let sprites = SpriteBank::load_default();
    let garden = showcase_garden();
    let viewport = Viewport::new(32, 10);

    for sprite_set in SpriteSet::ALL {
        let frame = render_garden(
            &garden,
            &catalog,
            &sprites,
            viewport,
            RenderOptions {
                sprites: sprite_set,
                color: ColorTier::NoColor,
                season: Season::Spring,
            },
        );
        assert_eq!(frame.layout(), LayoutKind::CompactList);
        let raw = frame.to_ansi_string();
        for line in raw.lines() {
            assert!(visible_width(line) <= usize::from(viewport.width));
        }
        let snap_name = format!("compact_list_32x10_{}", sprite_set.as_str());
        insta::assert_snapshot!(snap_name, raw);
    }
}
