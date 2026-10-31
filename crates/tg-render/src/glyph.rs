//! Single-cell compact glyph for prompt, tmux, snapshot, and macOS menu bar title (PLAN §4.3 & T07).

use tg_core::model::Stage;
use tg_core::sim::Garden;
use tg_core::species::SpriteSet;

/// Returns the [`Stage`] of the most advanced living plant in `garden`, or `None` when all plots are empty.
#[must_use]
pub fn most_advanced_stage(garden: &Garden) -> Option<Stage> {
    garden
        .plots()
        .iter()
        .filter_map(|plot| plot.plant().map(|plant| plant.stage))
        .max()
}

/// Maps `stage` (`None` = empty garden) to a strictly 1-cell, emoji-free glyph for `set` (PLAN §4.3).
#[must_use]
pub const fn stage_glyph(stage: Option<Stage>, set: SpriteSet) -> &'static str {
    match (set, stage) {
        (SpriteSet::Unicode, None) => "◌",
        (SpriteSet::Unicode, Some(Stage::Seed)) => "·",
        (SpriteSet::Unicode, Some(Stage::Sprout)) => "╷",
        (SpriteSet::Unicode, Some(Stage::Young)) => "┼",
        (SpriteSet::Unicode, Some(Stage::Mature)) => "◎",
        (SpriteSet::Unicode, Some(Stage::Blooming)) => "◈",
        (SpriteSet::Ascii, None) => "_",
        (SpriteSet::Ascii, Some(Stage::Seed)) => ".",
        (SpriteSet::Ascii, Some(Stage::Sprout)) => ",",
        (SpriteSet::Ascii, Some(Stage::Young)) => "+",
        (SpriteSet::Ascii, Some(Stage::Mature)) => "o",
        (SpriteSet::Ascii, Some(Stage::Blooming)) => "*",
    }
}

/// Computes the 1-cell compact glyph representing the most advanced plant in `garden`.
#[must_use]
pub fn compact_glyph(garden: &Garden, set: SpriteSet) -> &'static str {
    stage_glyph(most_advanced_stage(garden), set)
}

/// Emoji glyph reserved exclusively for `niwa prompt --style emoji` (PLAN §4.3 & T13).
///
/// Never used in TUI sprites, grid cells, or `SpriteSet::Unicode`.
#[must_use]
pub const fn prompt_emoji_glyph(stage: Option<Stage>) -> &'static str {
    match stage {
        None => "🪴",
        Some(Stage::Seed) => "🌰",
        Some(Stage::Sprout) => "🌱",
        Some(Stage::Young) => "🌿",
        Some(Stage::Mature) => "🌷",
        Some(Stage::Blooming) => "🌸",
    }
}
