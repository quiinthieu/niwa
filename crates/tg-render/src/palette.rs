//! Seasonal palettes (`Spring`, `Summer`, `Autumn`, `Winter`) and species `ColorHint` mapping (PLAN §4.2 & §4.3).

use crate::color::{Ansi16Color, StyleSpec};
use tg_core::model::Stage;
use tg_core::season::Season;
use tg_core::species::{ColorHint, Rarity};

/// Coordinated seasonal color palette covering borders, headers, ambient sky, foliage, and soil.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct SeasonPalette {
    pub season: Season,
    pub border: StyleSpec,
    pub header: StyleSpec,
    pub ambient: StyleSpec,
    pub foliage: StyleSpec,
    pub soil: StyleSpec,
    pub muted: StyleSpec,
    pub pot: StyleSpec,
}

impl SeasonPalette {
    #[must_use]
    pub const fn for_season(season: Season) -> Self {
        match season {
            Season::Spring => Self {
                season,
                border: StyleSpec::new((118, 196, 142), 114, Ansi16Color::Green),
                header: StyleSpec::new((242, 168, 196), 218, Ansi16Color::BrightMagenta)
                    .with_bold(),
                ambient: StyleSpec::new((246, 194, 214), 224, Ansi16Color::Magenta),
                foliage: StyleSpec::new((114, 210, 130), 114, Ansi16Color::BrightGreen),
                soil: StyleSpec::new((168, 138, 108), 137, Ansi16Color::Yellow),
                muted: StyleSpec::new((128, 144, 132), 245, Ansi16Color::BrightBlack).with_dim(),
                pot: StyleSpec::new((198, 138, 102), 173, Ansi16Color::Yellow),
            },
            Season::Summer => Self {
                season,
                border: StyleSpec::new((86, 192, 164), 79, Ansi16Color::Cyan),
                header: StyleSpec::new((248, 210, 96), 221, Ansi16Color::BrightYellow).with_bold(),
                ambient: StyleSpec::new((238, 224, 132), 228, Ansi16Color::BrightYellow),
                foliage: StyleSpec::new((76, 196, 118), 77, Ansi16Color::Green),
                soil: StyleSpec::new((172, 136, 98), 137, Ansi16Color::Yellow),
                muted: StyleSpec::new((124, 146, 140), 245, Ansi16Color::BrightBlack).with_dim(),
                pot: StyleSpec::new((206, 134, 92), 173, Ansi16Color::Red),
            },
            Season::Autumn => Self {
                season,
                border: StyleSpec::new((210, 140, 84), 173, Ansi16Color::Yellow),
                header: StyleSpec::new((238, 152, 78), 215, Ansi16Color::BrightRed).with_bold(),
                ambient: StyleSpec::new((216, 122, 74), 167, Ansi16Color::Red),
                foliage: StyleSpec::new((176, 164, 86), 143, Ansi16Color::Yellow),
                soil: StyleSpec::new((152, 118, 90), 137, Ansi16Color::Yellow),
                muted: StyleSpec::new((142, 128, 116), 244, Ansi16Color::BrightBlack).with_dim(),
                pot: StyleSpec::new((182, 114, 82), 131, Ansi16Color::Red),
            },
            Season::Winter => Self {
                season,
                border: StyleSpec::new((126, 170, 214), 110, Ansi16Color::Blue),
                header: StyleSpec::new((182, 222, 250), 153, Ansi16Color::BrightCyan).with_bold(),
                ambient: StyleSpec::new((206, 232, 252), 189, Ansi16Color::White),
                foliage: StyleSpec::new((96, 166, 144), 72, Ansi16Color::Cyan),
                soil: StyleSpec::new((138, 144, 156), 103, Ansi16Color::BrightBlack),
                muted: StyleSpec::new((122, 138, 156), 245, Ansi16Color::BrightBlack).with_dim(),
                pot: StyleSpec::new((158, 168, 184), 145, Ansi16Color::White),
            },
        }
    }
}

/// Maps a species [`ColorHint`] to its full-bloom highlight [`StyleSpec`].
#[must_use]
pub const fn hint_bloom_style(hint: ColorHint) -> StyleSpec {
    match hint {
        ColorHint::Green => StyleSpec::new((116, 226, 142), 114, Ansi16Color::BrightGreen),
        ColorHint::Pink => StyleSpec::new((246, 162, 198), 218, Ansi16Color::BrightMagenta),
        ColorHint::Gold => StyleSpec::new((248, 214, 102), 221, Ansi16Color::BrightYellow),
        ColorHint::Blue => StyleSpec::new((128, 196, 250), 117, Ansi16Color::BrightBlue),
        ColorHint::Violet => StyleSpec::new((192, 152, 246), 183, Ansi16Color::Magenta),
        ColorHint::Amber => StyleSpec::new((244, 168, 84), 215, Ansi16Color::Yellow),
        ColorHint::Frost => StyleSpec::new((214, 240, 252), 195, Ansi16Color::BrightCyan),
        ColorHint::Crimson => StyleSpec::new((238, 108, 122), 204, Ansi16Color::BrightRed),
    }
}

/// Resolves the sprite/stage style for a plant given its [`ColorHint`], [`Stage`], [`Season`], and dormancy.
#[must_use]
pub const fn plant_style(
    hint: ColorHint,
    stage: Stage,
    season: Season,
    dormant: bool,
) -> StyleSpec {
    let palette = SeasonPalette::for_season(season);
    if dormant {
        return match stage {
            Stage::Blooming => hint_bloom_style(hint).with_dim(),
            _ => palette.muted,
        };
    }
    match stage {
        Stage::Seed => palette.soil,
        Stage::Sprout | Stage::Young => palette.foliage,
        Stage::Mature => hint_bloom_style(hint),
        Stage::Blooming => hint_bloom_style(hint).with_bold(),
    }
}

/// Resolves the accent style for a [`Rarity`] label.
#[must_use]
pub const fn rarity_style(rarity: Rarity, season: Season) -> StyleSpec {
    let palette = SeasonPalette::for_season(season);
    match rarity {
        Rarity::Common => palette.muted,
        Rarity::Uncommon => StyleSpec::new((122, 208, 154), 114, Ansi16Color::Green),
        Rarity::Rare => StyleSpec::new((136, 194, 248), 117, Ansi16Color::BrightCyan),
        Rarity::Mythic => {
            StyleSpec::new((236, 174, 250), 219, Ansi16Color::BrightMagenta).with_bold()
        }
    }
}
