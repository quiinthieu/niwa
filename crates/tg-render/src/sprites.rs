//! Validated multi-line `unicode` and 7-bit `ascii` sprite bank loaded from `data/sprites/{unicode,ascii}/*.toml` (PLAN §4.3, §8 & T07).

use crate::width::display_width;
use serde::Deserialize;
use std::collections::BTreeMap;
use tg_core::model::Stage;
use tg_core::season::Season;
use tg_core::species::{is_forbidden_emoji_char, SpriteSet};
use thiserror::Error;
use toml::Spanned;

/// Height in rows of every multi-line plant stage sprite.
pub const SPRITE_ART_HEIGHT: usize = 3;
/// Width in terminal display columns of every multi-line plant stage sprite row.
pub const SPRITE_ART_WIDTH: usize = 7;

/// Compile-time embedded `data/sprites/unicode/*.toml` files (`(relative_path, toml_contents)`).
pub const EMBEDDED_UNICODE_SPRITES: &[(&str, &str)] = &[
    (
        "data/sprites/unicode/camellia.toml",
        include_str!("../../../data/sprites/unicode/camellia.toml"),
    ),
    (
        "data/sprites/unicode/chamomile.toml",
        include_str!("../../../data/sprites/unicode/chamomile.toml"),
    ),
    (
        "data/sprites/unicode/clover.toml",
        include_str!("../../../data/sprites/unicode/clover.toml"),
    ),
    (
        "data/sprites/unicode/daisy.toml",
        include_str!("../../../data/sprites/unicode/daisy.toml"),
    ),
    (
        "data/sprites/unicode/frostbell.toml",
        include_str!("../../../data/sprites/unicode/frostbell.toml"),
    ),
    (
        "data/sprites/unicode/lavender.toml",
        include_str!("../../../data/sprites/unicode/lavender.toml"),
    ),
    (
        "data/sprites/unicode/lily_of_the_valley.toml",
        include_str!("../../../data/sprites/unicode/lily_of_the_valley.toml"),
    ),
    (
        "data/sprites/unicode/lotus.toml",
        include_str!("../../../data/sprites/unicode/lotus.toml"),
    ),
    (
        "data/sprites/unicode/marigold.toml",
        include_str!("../../../data/sprites/unicode/marigold.toml"),
    ),
    (
        "data/sprites/unicode/moonflower.toml",
        include_str!("../../../data/sprites/unicode/moonflower.toml"),
    ),
    (
        "data/sprites/unicode/moss.toml",
        include_str!("../../../data/sprites/unicode/moss.toml"),
    ),
    (
        "data/sprites/unicode/primrose.toml",
        include_str!("../../../data/sprites/unicode/primrose.toml"),
    ),
    (
        "data/sprites/unicode/red_maple.toml",
        include_str!("../../../data/sprites/unicode/red_maple.toml"),
    ),
    (
        "data/sprites/unicode/sakura.toml",
        include_str!("../../../data/sprites/unicode/sakura.toml"),
    ),
    (
        "data/sprites/unicode/snowdrop.toml",
        include_str!("../../../data/sprites/unicode/snowdrop.toml"),
    ),
    (
        "data/sprites/unicode/sunburst_orchid.toml",
        include_str!("../../../data/sprites/unicode/sunburst_orchid.toml"),
    ),
];

/// Compile-time embedded `data/sprites/ascii/*.toml` files (`(relative_path, toml_contents)`).
pub const EMBEDDED_ASCII_SPRITES: &[(&str, &str)] = &[
    (
        "data/sprites/ascii/camellia.toml",
        include_str!("../../../data/sprites/ascii/camellia.toml"),
    ),
    (
        "data/sprites/ascii/chamomile.toml",
        include_str!("../../../data/sprites/ascii/chamomile.toml"),
    ),
    (
        "data/sprites/ascii/clover.toml",
        include_str!("../../../data/sprites/ascii/clover.toml"),
    ),
    (
        "data/sprites/ascii/daisy.toml",
        include_str!("../../../data/sprites/ascii/daisy.toml"),
    ),
    (
        "data/sprites/ascii/frostbell.toml",
        include_str!("../../../data/sprites/ascii/frostbell.toml"),
    ),
    (
        "data/sprites/ascii/lavender.toml",
        include_str!("../../../data/sprites/ascii/lavender.toml"),
    ),
    (
        "data/sprites/ascii/lily_of_the_valley.toml",
        include_str!("../../../data/sprites/ascii/lily_of_the_valley.toml"),
    ),
    (
        "data/sprites/ascii/lotus.toml",
        include_str!("../../../data/sprites/ascii/lotus.toml"),
    ),
    (
        "data/sprites/ascii/marigold.toml",
        include_str!("../../../data/sprites/ascii/marigold.toml"),
    ),
    (
        "data/sprites/ascii/moonflower.toml",
        include_str!("../../../data/sprites/ascii/moonflower.toml"),
    ),
    (
        "data/sprites/ascii/moss.toml",
        include_str!("../../../data/sprites/ascii/moss.toml"),
    ),
    (
        "data/sprites/ascii/primrose.toml",
        include_str!("../../../data/sprites/ascii/primrose.toml"),
    ),
    (
        "data/sprites/ascii/red_maple.toml",
        include_str!("../../../data/sprites/ascii/red_maple.toml"),
    ),
    (
        "data/sprites/ascii/sakura.toml",
        include_str!("../../../data/sprites/ascii/sakura.toml"),
    ),
    (
        "data/sprites/ascii/snowdrop.toml",
        include_str!("../../../data/sprites/ascii/snowdrop.toml"),
    ),
    (
        "data/sprites/ascii/sunburst_orchid.toml",
        include_str!("../../../data/sprites/ascii/sunburst_orchid.toml"),
    ),
];

/// Validated `3 × 7` multi-line sprite for a single growth [`Stage`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StageArt {
    rows: [String; SPRITE_ART_HEIGHT],
}

impl StageArt {
    #[must_use]
    pub const fn rows(&self) -> &[String; SPRITE_ART_HEIGHT] {
        &self.rows
    }

    #[must_use]
    pub fn as_str_rows(&self) -> [&str; SPRITE_ART_HEIGHT] {
        [
            self.rows[0].as_str(),
            self.rows[1].as_str(),
            self.rows[2].as_str(),
        ]
    }
}

/// Validated per-stage multi-line art for one species in one [`SpriteSet`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpeciesArt {
    pub species_id: String,
    pub seed: StageArt,
    pub sprout: StageArt,
    pub young: StageArt,
    pub mature: StageArt,
    pub blooming: StageArt,
}

impl SpeciesArt {
    #[must_use]
    pub const fn for_stage(&self, stage: Stage) -> &StageArt {
        match stage {
            Stage::Seed => &self.seed,
            Stage::Sprout => &self.sprout,
            Stage::Young => &self.young,
            Stage::Mature => &self.mature,
            Stage::Blooming => &self.blooming,
        }
    }
}

/// Location-rich error emitted when loading or validating a sprite TOML definition.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[error("{file}:{line}:{column}: {message}")]
pub struct SpriteLoadError {
    file: String,
    line: usize,
    column: usize,
    message: String,
}

impl SpriteLoadError {
    #[must_use]
    pub fn new(
        file: impl Into<String>,
        line: usize,
        column: usize,
        message: impl Into<String>,
    ) -> Self {
        Self {
            file: file.into(),
            line,
            column,
            message: message.into(),
        }
    }

    #[must_use]
    pub fn file(&self) -> &str {
        &self.file
    }

    #[must_use]
    pub const fn line(&self) -> usize {
        self.line
    }

    #[must_use]
    pub const fn column(&self) -> usize {
        self.column
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSpeciesArt {
    species_id: Spanned<String>,
    seed: Spanned<Vec<Spanned<String>>>,
    sprout: Spanned<Vec<Spanned<String>>>,
    young: Spanned<Vec<Spanned<String>>>,
    mature: Spanned<Vec<Spanned<String>>>,
    blooming: Spanned<Vec<Spanned<String>>>,
}

fn offset_to_line_col(source: &str, byte_offset: usize) -> (usize, usize) {
    let clamped = byte_offset.min(source.len());
    let prefix = &source[..clamped];
    let line = prefix.bytes().filter(|&b| b == b'\n').count() + 1;
    let line_start = prefix.rfind('\n').map_or(0, |idx| idx + 1);
    let column = source[line_start..clamped].chars().count() + 1;
    (line, column)
}

fn error_at(
    file: &str,
    source: &str,
    byte_offset: usize,
    message: impl Into<String>,
) -> SpriteLoadError {
    let (line, column) = offset_to_line_col(source, byte_offset);
    SpriteLoadError::new(file, line, column, message)
}

fn validate_stage_rows(
    file: &str,
    source: &str,
    set: SpriteSet,
    stage_name: &str,
    spanned_rows: Spanned<Vec<Spanned<String>>>,
) -> Result<StageArt, SpriteLoadError> {
    let outer_offset = spanned_rows.span().start;
    let rows_vec = spanned_rows.into_inner();
    if rows_vec.len() != SPRITE_ART_HEIGHT {
        return Err(error_at(
            file,
            source,
            outer_offset,
            format!(
                "{stage_name} must contain exactly {SPRITE_ART_HEIGHT} rows, got {}",
                rows_vec.len()
            ),
        ));
    }

    let mut validated: Vec<String> = Vec::with_capacity(SPRITE_ART_HEIGHT);
    for (row_idx, sp_row) in rows_vec.into_iter().enumerate() {
        let row_offset = sp_row.span().start;
        let row = sp_row.into_inner();

        match set {
            SpriteSet::Unicode => {
                if let Some(ch) = row.chars().find(|&c| is_forbidden_emoji_char(c)) {
                    return Err(error_at(
                        file,
                        source,
                        row_offset,
                        format!(
                            "{stage_name}[{row_idx}] contains forbidden emoji '{ch}' (PLAN §4.3)"
                        ),
                    ));
                }
                if row.chars().any(char::is_control) {
                    return Err(error_at(
                        file,
                        source,
                        row_offset,
                        format!("{stage_name}[{row_idx}] must not contain control characters"),
                    ));
                }
            }
            SpriteSet::Ascii => {
                if !row.bytes().all(|b| (0x20..=0x7E).contains(&b)) {
                    return Err(error_at(
                        file,
                        source,
                        row_offset,
                        format!(
                            "{stage_name}[{row_idx}] must contain only 7-bit printable ASCII characters"
                        ),
                    ));
                }
            }
        }

        let width = display_width(&row);
        if width != SPRITE_ART_WIDTH {
            return Err(error_at(
                file,
                source,
                row_offset,
                format!(
                    "{stage_name}[{row_idx}] must be {SPRITE_ART_WIDTH} display columns wide, got {width}"
                ),
            ));
        }
        validated.push(row);
    }

    let rows: [String; SPRITE_ART_HEIGHT] = validated
        .try_into()
        .expect("validated vector length equals SPRITE_ART_HEIGHT");
    Ok(StageArt { rows })
}

/// Complete bank of validated multi-line `unicode` and `ascii` plant sprites.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpriteBank {
    unicode: BTreeMap<String, SpeciesArt>,
    ascii: BTreeMap<String, SpeciesArt>,
}

impl SpriteBank {
    #[must_use]
    pub fn load_default() -> Self {
        Self::load_embedded()
            .expect("embedded data/sprites/{unicode,ascii}/*.toml files must be valid")
    }

    pub fn load_embedded() -> Result<Self, SpriteLoadError> {
        let mut unicode = BTreeMap::new();
        for &(file, raw) in EMBEDDED_UNICODE_SPRITES {
            let art = Self::parse_sprite_toml(file, SpriteSet::Unicode, raw)?;
            unicode.insert(art.species_id.clone(), art);
        }

        let mut ascii = BTreeMap::new();
        for &(file, raw) in EMBEDDED_ASCII_SPRITES {
            let art = Self::parse_sprite_toml(file, SpriteSet::Ascii, raw)?;
            ascii.insert(art.species_id.clone(), art);
        }

        Ok(Self { unicode, ascii })
    }

    pub fn parse_sprite_toml(
        file: &str,
        set: SpriteSet,
        raw: &str,
    ) -> Result<SpeciesArt, SpriteLoadError> {
        let parsed: RawSpeciesArt = toml::from_str(raw).map_err(|err| {
            let offset = err.span().map_or(raw.len(), |s| s.start);
            error_at(file, raw, offset, err.message())
        })?;

        let id_offset = parsed.species_id.span().start;
        let species_id = parsed.species_id.into_inner();
        if species_id.trim().is_empty() {
            return Err(error_at(
                file,
                raw,
                id_offset,
                "species_id must not be empty",
            ));
        }

        Ok(SpeciesArt {
            species_id,
            seed: validate_stage_rows(file, raw, set, "seed", parsed.seed)?,
            sprout: validate_stage_rows(file, raw, set, "sprout", parsed.sprout)?,
            young: validate_stage_rows(file, raw, set, "young", parsed.young)?,
            mature: validate_stage_rows(file, raw, set, "mature", parsed.mature)?,
            blooming: validate_stage_rows(file, raw, set, "blooming", parsed.blooming)?,
        })
    }

    /// Returns the `3 × 7` multi-line stage art for `(set, species_id, stage)`, falling back cleanly
    /// to a generic plant silhouette if `species_id` is a custom/unregistered species.
    #[must_use]
    pub fn art_for(
        &self,
        set: SpriteSet,
        species_id: &str,
        stage: Stage,
    ) -> [&str; SPRITE_ART_HEIGHT] {
        let map = match set {
            SpriteSet::Unicode => &self.unicode,
            SpriteSet::Ascii => &self.ascii,
        };
        map.get(species_id).map_or_else(
            || fallback_stage_art(set, stage),
            |sp| sp.for_stage(stage).as_str_rows(),
        )
    }
}

impl Default for SpriteBank {
    fn default() -> Self {
        Self::load_default()
    }
}

/// Returns the `3 × 7` resting soil art rendered inside empty plots.
#[must_use]
pub const fn empty_plot_art(set: SpriteSet) -> [&'static str; SPRITE_ART_HEIGHT] {
    match set {
        SpriteSet::Unicode => ["       ", " · · · ", " ───── "],
        SpriteSet::Ascii => ["       ", " . . . ", " ----- "],
    }
}

/// Returns a `7`-column planter pot rim placed beneath `3 × 7` sprites on standard cards.
#[must_use]
pub const fn pot_rim(set: SpriteSet) -> &'static str {
    match set {
        SpriteSet::Unicode => "╰─────╯",
        SpriteSet::Ascii => "\\_____/",
    }
}

/// Returns `(pot_top, pot_bottom)` (`9` columns each) placed beneath `3 × 7` sprites on spacious cards.
#[must_use]
pub const fn wide_pot(set: SpriteSet) -> (&'static str, &'static str) {
    match set {
        SpriteSet::Unicode => ("╭───────╮", "╰───────╯"),
        SpriteSet::Ascii => ("/-------\\", "\\_______/"),
    }
}

/// Returns the seasonal ambient sky motif (`petals`, `fireflies`, `leaves`, `snow`; PLAN §4.2).
#[must_use]
pub const fn seasonal_ambient_banner(set: SpriteSet, season: Season) -> &'static str {
    match (set, season) {
        (SpriteSet::Unicode, Season::Spring) => "· ∘ · ░ spring petals drifting · ∘ ·",
        (SpriteSet::Unicode, Season::Summer) => "· • · ◈ summer fireflies glowing · • ·",
        (SpriteSet::Unicode, Season::Autumn) => "· ◇ · ▒ autumn leaves rustling · ◇ ·",
        (SpriteSet::Unicode, Season::Winter) => "· ° · ◌ winter snow settling · ° ·",
        (SpriteSet::Ascii, Season::Spring) => ". o . ~ spring petals drifting . o .",
        (SpriteSet::Ascii, Season::Summer) => ". * . + summer fireflies glowing . * .",
        (SpriteSet::Ascii, Season::Autumn) => ". ~ . / autumn leaves rustling . ~ .",
        (SpriteSet::Ascii, Season::Winter) => ". * . . winter snow settling . * .",
    }
}

const fn fallback_stage_art(set: SpriteSet, stage: Stage) -> [&'static str; SPRITE_ART_HEIGHT] {
    match (set, stage) {
        (SpriteSet::Unicode, Stage::Seed) => ["       ", "       ", "  ·•·  "],
        (SpriteSet::Unicode, Stage::Sprout) => ["       ", "   ╷   ", "  ─┴─  "],
        (SpriteSet::Unicode, Stage::Young) => ["   ╷   ", "  ├│┤  ", "  ─┴─  "],
        (SpriteSet::Unicode, Stage::Mature) => ["   ◎   ", "  ├│┤  ", "  ─┴─  "],
        (SpriteSet::Unicode, Stage::Blooming) => [" ░▓◈▓░ ", "  ├│┤  ", "  ─┴─  "],
        (SpriteSet::Ascii, Stage::Seed) => ["       ", "       ", "  ...  "],
        (SpriteSet::Ascii, Stage::Sprout) => ["       ", "   ,   ", "  -+-  "],
        (SpriteSet::Ascii, Stage::Young) => ["   |   ", "  -|-  ", "  -+-  "],
        (SpriteSet::Ascii, Stage::Mature) => ["   o   ", "  -|-  ", "  -+-  "],
        (SpriteSet::Ascii, Stage::Blooming) => [" .*@*. ", "  -|-  ", "  -+-  "],
    }
}
