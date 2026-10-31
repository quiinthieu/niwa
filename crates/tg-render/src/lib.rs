//! tg-render: Pure garden frame renderer (Unicode and ASCII sprite sets, seasonal color tiers,
//! `NO_COLOR`, responsive grid + compact list fallback, and single-cell compact glyph).

pub mod color;
pub mod glyph;
pub mod layout;
pub mod palette;
pub mod sprites;
pub mod width;

pub use color::{detect_color_tier, ColorEnv, ColorPreference, ColorTier, StyleSpec};
pub use glyph::{
    compact_glyph, most_advanced_stage, prompt_emoji_glyph, stage_from_glyph, stage_glyph,
};
pub use layout::{
    render_compact_frame, render_garden, LayoutKind, RenderOptions, RenderedFrame, RenderedLine,
    StyledSpan, Viewport, MIN_GRID_HEIGHT, MIN_GRID_WIDTH,
};
pub use palette::{plant_style, rarity_style, SeasonPalette};
pub use sprites::{SpriteBank, SpriteLoadError, SPRITE_ART_HEIGHT, SPRITE_ART_WIDTH};
pub use tg_core::species::SpriteSet;
