//! Species definitions, rarity tiers, and validated `data/species/*.toml` loader (PLAN §4.2 & T04).

use crate::balance::{multiplier_to_bp, GrowthProfile, MULTIPLIER_BP_SCALE};
use crate::model::Stage;
use crate::season::Season;
use crate::sim::ProfileResolver;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;
use toml::Spanned;

/// Rarity tiers for plants and seed drops (PLAN §4.2).
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rarity {
    Common,
    Uncommon,
    Rare,
    Mythic,
}

impl Rarity {
    pub const ALL: [Self; 4] = [Self::Common, Self::Uncommon, Self::Rare, Self::Mythic];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Common => "common",
            Self::Uncommon => "uncommon",
            Self::Rare => "rare",
            Self::Mythic => "mythic",
        }
    }
}

/// Palette color hint used by `tg-render` (PLAN §4.2 & §4.3).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorHint {
    Green,
    Pink,
    Gold,
    Blue,
    Violet,
    Amber,
    Frost,
    Crimson,
}

/// Per-stage sprite strings for one sprite set (`unicode` or `ascii`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageSprites {
    pub seed: String,
    pub sprout: String,
    pub young: String,
    pub mature: String,
    pub blooming: String,
}

impl StageSprites {
    #[must_use]
    pub fn for_stage(&self, stage: Stage) -> &str {
        match stage {
            Stage::Seed => &self.seed,
            Stage::Sprout => &self.sprout,
            Stage::Young => &self.young,
            Stage::Mature => &self.mature,
            Stage::Blooming => &self.blooming,
        }
    }
}

/// Paired `unicode` and 7-bit `ascii` sprites for a species.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpeciesSprites {
    pub unicode: StageSprites,
    pub ascii: StageSprites,
}

/// Validated species definition loaded from `data/species/*.toml`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Species {
    pub id: String,
    pub display_name: String,
    pub rarity: Rarity,
    pub gp_per_stage: [u64; 4],
    pub growth_multiplier_bp: u32,
    pub preferred_seasons: Vec<Season>,
    pub seasonal_sprout_only: bool,
    pub color_hint: ColorHint,
    pub almanac_flavour: String,
    pub sprites: SpeciesSprites,
}

impl Species {
    #[must_use]
    pub fn prefers_season(&self, season: Season) -> bool {
        self.preferred_seasons.contains(&season)
    }

    #[must_use]
    pub fn can_sprout_in(&self, season: Season) -> bool {
        !self.seasonal_sprout_only || self.prefers_season(season)
    }

    #[must_use]
    pub const fn growth_profile(&self) -> GrowthProfile {
        GrowthProfile {
            stage_thresholds: self.gp_per_stage,
            species_multiplier_bp: self.growth_multiplier_bp,
        }
    }
}

/// Compile-time embedded `data/species/*.toml` sources (`(relative_path, toml_contents)`).
pub const EMBEDDED_SPECIES_FILES: &[(&str, &str)] = &[
    (
        "data/species/camellia.toml",
        include_str!("../../../data/species/camellia.toml"),
    ),
    (
        "data/species/chamomile.toml",
        include_str!("../../../data/species/chamomile.toml"),
    ),
    (
        "data/species/clover.toml",
        include_str!("../../../data/species/clover.toml"),
    ),
    (
        "data/species/daisy.toml",
        include_str!("../../../data/species/daisy.toml"),
    ),
    (
        "data/species/frostbell.toml",
        include_str!("../../../data/species/frostbell.toml"),
    ),
    (
        "data/species/lavender.toml",
        include_str!("../../../data/species/lavender.toml"),
    ),
    (
        "data/species/lily_of_the_valley.toml",
        include_str!("../../../data/species/lily_of_the_valley.toml"),
    ),
    (
        "data/species/lotus.toml",
        include_str!("../../../data/species/lotus.toml"),
    ),
    (
        "data/species/marigold.toml",
        include_str!("../../../data/species/marigold.toml"),
    ),
    (
        "data/species/moonflower.toml",
        include_str!("../../../data/species/moonflower.toml"),
    ),
    (
        "data/species/moss.toml",
        include_str!("../../../data/species/moss.toml"),
    ),
    (
        "data/species/primrose.toml",
        include_str!("../../../data/species/primrose.toml"),
    ),
    (
        "data/species/red_maple.toml",
        include_str!("../../../data/species/red_maple.toml"),
    ),
    (
        "data/species/sakura.toml",
        include_str!("../../../data/species/sakura.toml"),
    ),
    (
        "data/species/snowdrop.toml",
        include_str!("../../../data/species/snowdrop.toml"),
    ),
    (
        "data/species/sunburst_orchid.toml",
        include_str!("../../../data/species/sunburst_orchid.toml"),
    ),
];

/// Location-rich error emitted when loading or validating a species TOML file.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[error("{file}:{line}:{column}: {message}")]
pub struct SpeciesLoadError {
    file: String,
    line: usize,
    column: usize,
    message: String,
}

impl SpeciesLoadError {
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
struct RawSpecies {
    id: Spanned<String>,
    display_name: Spanned<String>,
    rarity: Rarity,
    gp_per_stage: [Spanned<u64>; 4],
    #[serde(default)]
    growth_multiplier: Option<Spanned<f64>>,
    preferred_seasons: Spanned<Vec<Spanned<Season>>>,
    #[serde(default)]
    seasonal_sprout_only: bool,
    color_hint: ColorHint,
    almanac_flavour: Spanned<String>,
    sprites: RawSpeciesSprites,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSpeciesSprites {
    unicode: RawStageSprites,
    ascii: RawStageSprites,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStageSprites {
    seed: Spanned<String>,
    sprout: Spanned<String>,
    young: Spanned<String>,
    mature: Spanned<String>,
    blooming: Spanned<String>,
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
) -> SpeciesLoadError {
    let (line, column) = offset_to_line_col(source, byte_offset);
    SpeciesLoadError::new(file, line, column, message)
}

fn is_valid_species_id(id: &str) -> bool {
    let mut bytes = id.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    first.is_ascii_lowercase()
        && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn is_forbidden_emoji_char(ch: char) -> bool {
    let code = u32::from(ch);
    (0xFE0E..=0xFE0F).contains(&code)
        || (0x2600..=0x27BF).contains(&code)
        || (0x1F000..=0x1FAFF).contains(&code)
}

fn validate_unicode_sprites(
    file: &str,
    source: &str,
    raw: &RawStageSprites,
) -> Result<StageSprites, SpeciesLoadError> {
    let fields = [
        ("sprites.unicode.seed", &raw.seed),
        ("sprites.unicode.sprout", &raw.sprout),
        ("sprites.unicode.young", &raw.young),
        ("sprites.unicode.mature", &raw.mature),
        ("sprites.unicode.blooming", &raw.blooming),
    ];
    for (name, field) in fields {
        let value = field.get_ref();
        if value.is_empty() {
            return Err(error_at(
                file,
                source,
                field.span().start,
                format!("{name} must not be empty"),
            ));
        }
        if let Some(ch) = value.chars().find(|&c| is_forbidden_emoji_char(c)) {
            return Err(error_at(
                file,
                source,
                field.span().start,
                format!(
                    "{name} contains emoji character '{ch}' (PLAN §4.3 forbids emoji in sprites)"
                ),
            ));
        }
    }
    Ok(StageSprites {
        seed: raw.seed.get_ref().clone(),
        sprout: raw.sprout.get_ref().clone(),
        young: raw.young.get_ref().clone(),
        mature: raw.mature.get_ref().clone(),
        blooming: raw.blooming.get_ref().clone(),
    })
}

fn validate_ascii_sprites(
    file: &str,
    source: &str,
    raw: &RawStageSprites,
) -> Result<StageSprites, SpeciesLoadError> {
    let fields = [
        ("sprites.ascii.seed", &raw.seed),
        ("sprites.ascii.sprout", &raw.sprout),
        ("sprites.ascii.young", &raw.young),
        ("sprites.ascii.mature", &raw.mature),
        ("sprites.ascii.blooming", &raw.blooming),
    ];
    for (name, field) in fields {
        let value = field.get_ref();
        if value.is_empty() {
            return Err(error_at(
                file,
                source,
                field.span().start,
                format!("{name} must not be empty"),
            ));
        }
        if !value
            .bytes()
            .all(|b| b == b'\n' || (0x20..=0x7E).contains(&b))
        {
            return Err(error_at(
                file,
                source,
                field.span().start,
                format!("{name} must contain only 7-bit printable ASCII characters"),
            ));
        }
    }
    Ok(StageSprites {
        seed: raw.seed.get_ref().clone(),
        sprout: raw.sprout.get_ref().clone(),
        young: raw.young.get_ref().clone(),
        mature: raw.mature.get_ref().clone(),
        blooming: raw.blooming.get_ref().clone(),
    })
}

fn parse_species_with_id_loc(
    file: &str,
    raw: &str,
) -> Result<(Species, (usize, usize)), SpeciesLoadError> {
    let parsed: RawSpecies = toml::from_str(raw).map_err(|err| {
        let offset = err.span().map_or(raw.len(), |s| s.start);
        error_at(file, raw, offset, err.message())
    })?;

    let id_offset = parsed.id.span().start;
    let id = parsed.id.into_inner();
    if !is_valid_species_id(&id) {
        return Err(error_at(
            file,
            raw,
            id_offset,
            format!("species id '{id}' must be a lowercase ASCII slug ([a-z][a-z0-9_]*)"),
        ));
    }

    let name_offset = parsed.display_name.span().start;
    let display_name = parsed.display_name.into_inner();
    if display_name.trim().is_empty() {
        return Err(error_at(
            file,
            raw,
            name_offset,
            "display_name must not be empty",
        ));
    }

    let mut gp_per_stage = [0_u64; 4];
    for (idx, spanned_val) in parsed.gp_per_stage.into_iter().enumerate() {
        let offset = spanned_val.span().start;
        let val = spanned_val.into_inner();
        if val == 0 {
            return Err(error_at(
                file,
                raw,
                offset,
                format!("gp_per_stage[{idx}] must be >= 1, got 0"),
            ));
        }
        gp_per_stage[idx] = val;
    }

    let growth_multiplier_bp = match parsed.growth_multiplier {
        Some(spanned_mult) => {
            let mult_offset = spanned_mult.span().start;
            let raw_mult = spanned_mult.into_inner();
            multiplier_to_bp("growth_multiplier", raw_mult)
                .ok()
                .filter(|&bp| bp > 0)
                .ok_or_else(|| {
                    error_at(
                        file,
                        raw,
                        mult_offset,
                        "growth_multiplier must be finite and strictly positive",
                    )
                })?
        }
        None => MULTIPLIER_BP_SCALE,
    };

    let seasons_offset = parsed.preferred_seasons.span().start;
    let spanned_seasons = parsed.preferred_seasons.into_inner();
    if spanned_seasons.is_empty() {
        return Err(error_at(
            file,
            raw,
            seasons_offset,
            "preferred_seasons must list at least one season",
        ));
    }
    let mut seen_seasons = BTreeSet::new();
    let mut preferred_seasons = Vec::with_capacity(spanned_seasons.len());
    for sp_season in spanned_seasons {
        let s_offset = sp_season.span().start;
        let season = sp_season.into_inner();
        if !seen_seasons.insert(season) {
            return Err(error_at(
                file,
                raw,
                s_offset,
                format!(
                    "duplicate season '{}' in preferred_seasons",
                    season.as_str()
                ),
            ));
        }
        preferred_seasons.push(season);
    }

    let flavour_offset = parsed.almanac_flavour.span().start;
    let almanac_flavour = parsed.almanac_flavour.into_inner();
    if almanac_flavour.trim().is_empty() {
        return Err(error_at(
            file,
            raw,
            flavour_offset,
            "almanac_flavour must not be empty",
        ));
    }

    let unicode = validate_unicode_sprites(file, raw, &parsed.sprites.unicode)?;
    let ascii = validate_ascii_sprites(file, raw, &parsed.sprites.ascii)?;

    let species = Species {
        id,
        display_name,
        rarity: parsed.rarity,
        gp_per_stage,
        growth_multiplier_bp,
        preferred_seasons,
        seasonal_sprout_only: parsed.seasonal_sprout_only,
        color_hint: parsed.color_hint,
        almanac_flavour,
        sprites: SpeciesSprites { unicode, ascii },
    };

    Ok((species, offset_to_line_col(raw, id_offset)))
}

/// Deterministic, sorted catalog of validated [`Species`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpeciesCatalog {
    by_id: BTreeMap<String, Species>,
}

impl SpeciesCatalog {
    #[must_use]
    pub fn load_default() -> Self {
        Self::load_embedded().expect("embedded data/species/*.toml files must be valid")
    }

    pub fn load_embedded() -> Result<Self, SpeciesLoadError> {
        Self::from_named_sources(EMBEDDED_SPECIES_FILES.iter().copied())
    }

    pub fn from_named_sources<'a, I>(sources: I) -> Result<Self, SpeciesLoadError>
    where
        I: IntoIterator<Item = (&'a str, &'a str)>,
    {
        let mut by_id: BTreeMap<String, Species> = BTreeMap::new();
        let mut defined_in: BTreeMap<String, String> = BTreeMap::new();

        for (file, raw) in sources {
            let (species, (id_line, id_col)) = parse_species_with_id_loc(file, raw)?;
            if let Some(prev_file) = defined_in.get(&species.id) {
                return Err(SpeciesLoadError::new(
                    file,
                    id_line,
                    id_col,
                    format!(
                        "duplicate species id '{}' (already defined in {prev_file})",
                        species.id
                    ),
                ));
            }
            defined_in.insert(species.id.clone(), file.to_string());
            by_id.insert(species.id.clone(), species);
        }

        Ok(Self { by_id })
    }

    pub fn parse_species_toml(file: &str, raw: &str) -> Result<Species, SpeciesLoadError> {
        parse_species_with_id_loc(file, raw).map(|(sp, _)| sp)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Species> {
        self.by_id.get(id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Species> {
        self.by_id.values()
    }
}

impl Default for SpeciesCatalog {
    fn default() -> Self {
        Self::load_default()
    }
}

impl ProfileResolver for (&crate::balance::Balance, &SpeciesCatalog) {
    fn profile_for(&self, species_id: &str) -> GrowthProfile {
        self.1
            .get(species_id)
            .map_or_else(|| self.0.default_profile.clone(), Species::growth_profile)
    }
}
