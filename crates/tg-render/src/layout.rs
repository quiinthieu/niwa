//! Responsive garden frame renderer: grid layout (`>= 40×12`) + compact list fallback (`< 40×12`) (PLAN §4.3 & T07).

use crate::color::{ColorTier, StyleSpec};
use crate::glyph::compact_glyph;
use crate::palette::{plant_style, rarity_style, SeasonPalette};
use crate::sprites::{
    empty_plot_art, pot_rim, seasonal_ambient_banner_frame, seasonal_canopy_aura,
    seasonal_cell_motes, seasonal_header_tag, wide_pot, SpriteBank, SPRITE_ART_WIDTH,
};
use crate::width::{display_width, pad_center_to_width, pad_right_to_width, truncate_to_width};
use tg_core::model::{Plant, Plot, Stage};
use tg_core::product;
use tg_core::season::Season;
use tg_core::sim::Garden;
use tg_core::species::{ColorHint, Rarity, Species, SpeciesCatalog, SpriteSet};

/// Minimum terminal width in columns required for the grid layout (PLAN §4.3).
pub const MIN_GRID_WIDTH: u16 = 40;
/// Minimum terminal height in rows required for the grid layout (PLAN §4.3).
pub const MIN_GRID_HEIGHT: u16 = 12;
const MIN_CELL_INTERIOR_WIDTH: usize = 10;
const MIN_CELL_INTERIOR_HEIGHT: usize = 2;

/// Target terminal viewport dimensions (`width` columns × `height` rows).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Viewport {
    pub width: u16,
    pub height: u16,
}

impl Viewport {
    #[must_use]
    pub const fn new(width: u16, height: u16) -> Self {
        Self { width, height }
    }
}

/// Active layout chosen for a [`RenderedFrame`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum LayoutKind {
    Grid,
    CompactList,
}

/// Pure rendering parameters (`sprites`, `color`, `season`).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct RenderOptions {
    pub sprites: SpriteSet,
    pub color: ColorTier,
    pub season: Season,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            sprites: SpriteSet::Unicode,
            color: ColorTier::NoColor,
            season: Season::Spring,
        }
    }
}

/// Styled substring span within a [`RenderedLine`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StyledSpan {
    pub text: String,
    pub style: Option<StyleSpec>,
}

impl StyledSpan {
    #[must_use]
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            style: None,
        }
    }

    #[must_use]
    pub fn styled(text: impl Into<String>, style: StyleSpec) -> Self {
        Self {
            text: text.into(),
            style: Some(style),
        }
    }
}

/// Single rendered row guaranteed to fit within the frame's [`Viewport::width`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RenderedLine {
    spans: Vec<StyledSpan>,
    width: usize,
}

impl RenderedLine {
    #[must_use]
    pub fn spans(&self) -> &[StyledSpan] {
        &self.spans
    }

    #[must_use]
    pub const fn display_width(&self) -> usize {
        self.width
    }

    #[must_use]
    pub fn plain_text(&self) -> String {
        let mut out = String::with_capacity(self.width);
        for span in &self.spans {
            out.push_str(&span.text);
        }
        out
    }

    #[must_use]
    pub fn to_ansi_string(&self, tier: ColorTier) -> String {
        if tier == ColorTier::NoColor {
            return self.plain_text();
        }
        let mut out = String::with_capacity(self.width + self.spans.len() * 16);
        for span in &self.spans {
            if span.text.is_empty() {
                continue;
            }
            if let Some(style) = span.style {
                if span.text.bytes().all(|b| b == b' ') {
                    out.push_str(&span.text);
                } else {
                    style.write_sgr_open(tier, &mut out);
                    out.push_str(&span.text);
                    out.push_str("\x1b[0m");
                }
            } else {
                out.push_str(&span.text);
            }
        }
        out
    }
}

/// Complete rendered frame produced by [`render_garden`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderedFrame {
    viewport: Viewport,
    layout: LayoutKind,
    color: ColorTier,
    lines: Vec<RenderedLine>,
}

impl RenderedFrame {
    #[must_use]
    pub const fn viewport(&self) -> Viewport {
        self.viewport
    }

    #[must_use]
    pub const fn layout(&self) -> LayoutKind {
        self.layout
    }

    #[must_use]
    pub fn lines(&self) -> &[RenderedLine] {
        &self.lines
    }

    #[must_use]
    pub fn to_plain_string(&self) -> String {
        self.render_for_tier(ColorTier::NoColor)
    }

    #[must_use]
    pub fn to_ansi_string(&self) -> String {
        self.render_for_tier(self.color)
    }

    #[must_use]
    pub fn render_for_tier(&self, tier: ColorTier) -> String {
        let mut out = String::new();
        for (idx, line) in self.lines.iter().enumerate() {
            if idx > 0 {
                out.push('\n');
            }
            out.push_str(&line.to_ansi_string(tier));
        }
        out
    }
}

#[derive(Default)]
struct LineBuilder {
    spans: Vec<StyledSpan>,
    width: usize,
}

impl LineBuilder {
    fn push_plain(&mut self, text: impl Into<String>) {
        let s = text.into();
        if !s.is_empty() {
            self.width += display_width(&s);
            self.spans.push(StyledSpan::plain(s));
        }
    }

    fn push_styled(&mut self, text: impl Into<String>, style: StyleSpec) {
        let s = text.into();
        if !s.is_empty() {
            self.width += display_width(&s);
            self.spans.push(StyledSpan::styled(s, style));
        }
    }

    fn append_fitted(&mut self, other: LineBuilder, target_width: usize) {
        let fitted = other.into_fitted(target_width, true);
        self.width += fitted.width;
        self.spans.extend(fitted.spans);
    }

    fn into_fitted(self, max_width: usize, pad_right: bool) -> RenderedLine {
        if max_width == 0 {
            return RenderedLine::default();
        }
        let mut spans = Vec::with_capacity(self.spans.len() + 1);
        let mut used = 0;

        for span in self.spans {
            if used >= max_width {
                break;
            }
            let remaining = max_width - used;
            let sw = display_width(&span.text);
            if sw <= remaining {
                if !span.text.is_empty() {
                    used += sw;
                    spans.push(span);
                }
            } else {
                let clipped = truncate_to_width(&span.text, remaining);
                let cw = display_width(&clipped);
                if cw > 0 {
                    used += cw;
                    spans.push(StyledSpan {
                        text: clipped,
                        style: span.style,
                    });
                }
                break;
            }
        }

        if pad_right && used < max_width {
            spans.push(StyledSpan::plain(" ".repeat(max_width - used)));
            used = max_width;
        }

        RenderedLine { spans, width: used }
    }
}

#[derive(Copy, Clone)]
struct BoxChars {
    h: &'static str,
    v: &'static str,
    tl: &'static str,
    tm: &'static str,
    tr: &'static str,
    ml: &'static str,
    mm: &'static str,
    mr: &'static str,
    bl: &'static str,
    bm: &'static str,
    br: &'static str,
    sep: &'static str,
    bar_fill: char,
    bar_empty: char,
    bloom_mark: &'static str,
}

impl BoxChars {
    const fn for_set(set: SpriteSet) -> Self {
        match set {
            SpriteSet::Unicode => Self {
                h: "─",
                v: "│",
                tl: "┌",
                tm: "┬",
                tr: "┐",
                ml: "├",
                mm: "┼",
                mr: "┤",
                bl: "└",
                bm: "┴",
                br: "┘",
                sep: "·",
                bar_fill: '█',
                bar_empty: '░',
                bloom_mark: "◈",
            },
            SpriteSet::Ascii => Self {
                h: "-",
                v: "|",
                tl: "+",
                tm: "+",
                tr: "+",
                ml: "+",
                mm: "+",
                mr: "+",
                bl: "+",
                bm: "+",
                br: "+",
                sep: "-",
                bar_fill: '#',
                bar_empty: '.',
                bloom_mark: "*",
            },
        }
    }
}

/// Renders a [`Garden`] into a pure, width-safe [`RenderedFrame`] with no terminal I/O at resting phase `0`.
#[must_use]
pub fn render_garden(
    garden: &Garden,
    catalog: &SpeciesCatalog,
    sprites: &SpriteBank,
    viewport: Viewport,
    options: RenderOptions,
) -> RenderedFrame {
    render_garden_with_phase(garden, catalog, sprites, viewport, options, 0)
}

/// Renders a [`Garden`] into a pure, width-safe [`RenderedFrame`] with seasonal ambient particle
/// `phase` (`0..4`, resting at `0` whenever `garden.is_dormant()` is true; PLAN §4.2, §4.3 & §10 T16).
#[must_use]
pub fn render_garden_with_phase(
    garden: &Garden,
    catalog: &SpeciesCatalog,
    sprites: &SpriteBank,
    viewport: Viewport,
    options: RenderOptions,
    phase: u8,
) -> RenderedFrame {
    let w = usize::from(viewport.width);
    let h = usize::from(viewport.height);
    let effective_phase = if garden.is_dormant() { 0 } else { phase % 4 };

    if w == 0 || h == 0 {
        let layout = if viewport.width >= MIN_GRID_WIDTH && viewport.height >= MIN_GRID_HEIGHT {
            LayoutKind::Grid
        } else {
            LayoutKind::CompactList
        };
        return RenderedFrame {
            viewport,
            layout,
            color: options.color,
            lines: Vec::new(),
        };
    }

    if viewport.width >= MIN_GRID_WIDTH && viewport.height >= MIN_GRID_HEIGHT {
        if let Some(lines) =
            try_render_grid(garden, catalog, sprites, w, h, options, effective_phase)
        {
            return RenderedFrame {
                viewport,
                layout: LayoutKind::Grid,
                color: options.color,
                lines,
            };
        }
    }

    let lines = render_compact_list(garden, catalog, w, h, options);
    RenderedFrame {
        viewport,
        layout: LayoutKind::CompactList,
        color: options.color,
        lines,
    }
}

/// Renders a plain-text `40×12` compact frame suitable for persisting in `snapshot.json` (PLAN §6 & T14).
#[must_use]
pub fn render_compact_frame(
    garden: &Garden,
    catalog: &SpeciesCatalog,
    sprites: &SpriteBank,
    set: SpriteSet,
    season: Season,
) -> String {
    render_garden(
        garden,
        catalog,
        sprites,
        Viewport::new(MIN_GRID_WIDTH, MIN_GRID_HEIGHT),
        RenderOptions {
            sprites: set,
            color: ColorTier::NoColor,
            season,
        },
    )
    .to_plain_string()
}

fn try_render_grid(
    garden: &Garden,
    catalog: &SpeciesCatalog,
    sprites: &SpriteBank,
    width: usize,
    height: usize,
    options: RenderOptions,
    phase: u8,
) -> Option<Vec<RenderedLine>> {
    let plot_count = garden.plots().len().max(1);
    let mut cols = match plot_count {
        1 => 1,
        2..=4 => 2,
        5..=9 => 3,
        _ => 4,
    };

    while cols > 1 && width.saturating_sub(cols + 1) / cols < MIN_CELL_INTERIOR_WIDTH {
        cols -= 1;
    }
    if width.saturating_sub(cols + 1) < cols * MIN_CELL_INTERIOR_WIDTH {
        return None;
    }

    let rows = plot_count.div_ceil(cols);
    let show_ambient_banner = height >= 30;
    let chrome_rows = 2 + usize::from(show_ambient_banner);
    let border_rows = rows + 1;
    if height < chrome_rows + border_rows + rows * MIN_CELL_INTERIOR_HEIGHT {
        return None;
    }

    let interior_h = height - chrome_rows - border_rows;
    let cell_h = interior_h / rows;
    let extra_spacer_rows = interior_h % rows;

    let interior_w = width - (cols + 1);
    let base_col_w = interior_w / cols;
    let rem_col_w = interior_w % cols;
    let col_widths: Vec<usize> = (0..cols)
        .map(|c| base_col_w + usize::from(c < rem_col_w))
        .collect();

    let palette = SeasonPalette::for_season(options.season);
    let box_chars = BoxChars::for_set(options.sprites);
    let mut out = Vec::with_capacity(height);

    // 1. Top header row.
    out.push(build_header_line(
        garden, width, options, &palette, &box_chars, phase,
    ));

    // 2. Optional seasonal ambient sky row on tall terminals (e.g. 120x40).
    if show_ambient_banner {
        let mut ambient_lb = LineBuilder::default();
        let banner = pad_center_to_width(
            seasonal_ambient_banner_frame(options.sprites, options.season, phase),
            width,
        );
        ambient_lb.push_styled(
            banner,
            if garden.is_dormant() {
                palette.muted
            } else {
                palette.ambient
            },
        );
        out.push(ambient_lb.into_fitted(width, true));
    }

    // 3. Top grid border.
    out.push(build_horizontal_border(
        &col_widths,
        box_chars.tl,
        box_chars.tm,
        box_chars.tr,
        box_chars.h,
        palette.border,
        width,
    ));

    // 4. Grid rows.
    for r in 0..rows {
        let mut cell_blocks: Vec<Vec<LineBuilder>> = Vec::with_capacity(cols);
        for (c, &cw) in col_widths.iter().enumerate() {
            let plot_idx = r * cols + c;
            let block = if let Some(plot) = garden.plots().get(plot_idx) {
                render_plot_cell(
                    plot_idx,
                    plot,
                    garden.is_dormant(),
                    catalog,
                    sprites,
                    cw,
                    cell_h,
                    options,
                    &palette,
                    &box_chars,
                    phase,
                )
            } else {
                (0..cell_h).map(|_| LineBuilder::default()).collect()
            };
            cell_blocks.push(block);
        }

        for line_idx in 0..cell_h {
            let mut row_lb = LineBuilder::default();
            row_lb.push_styled(box_chars.v, palette.border);
            for (block, &cw) in cell_blocks.iter_mut().zip(col_widths.iter()) {
                let cell_line = std::mem::take(&mut block[line_idx]);
                row_lb.append_fitted(cell_line, cw);
                row_lb.push_styled(box_chars.v, palette.border);
            }
            out.push(row_lb.into_fitted(width, true));
        }

        if r + 1 < rows {
            out.push(build_horizontal_border(
                &col_widths,
                box_chars.ml,
                box_chars.mm,
                box_chars.mr,
                box_chars.h,
                palette.border,
                width,
            ));
        } else {
            out.push(build_horizontal_border(
                &col_widths,
                box_chars.bl,
                box_chars.bm,
                box_chars.br,
                box_chars.h,
                palette.border,
                width,
            ));
        }
    }

    // 5. Optional remainder spacer lines so total frame height matches `height` exactly.
    for _ in 0..extra_spacer_rows {
        out.push(LineBuilder::default().into_fitted(width, true));
    }

    // 6. Footer status bar.
    out.push(build_footer_line(
        garden, width, options, &palette, &box_chars,
    ));

    debug_assert_eq!(out.len(), height);
    Some(out)
}

fn build_horizontal_border(
    col_widths: &[usize],
    left: &str,
    mid: &str,
    right: &str,
    horiz: &str,
    style: StyleSpec,
    max_width: usize,
) -> RenderedLine {
    let mut s = String::with_capacity(max_width * 3);
    s.push_str(left);
    for (idx, &cw) in col_widths.iter().enumerate() {
        if idx > 0 {
            s.push_str(mid);
        }
        for _ in 0..cw {
            s.push_str(horiz);
        }
    }
    s.push_str(right);
    let mut lb = LineBuilder::default();
    lb.push_styled(s, style);
    lb.into_fitted(max_width, true)
}

fn build_header_line(
    garden: &Garden,
    width: usize,
    options: RenderOptions,
    palette: &SeasonPalette,
    box_chars: &BoxChars,
    phase: u8,
) -> RenderedLine {
    let total_plots = garden.plots().len();
    let planted = garden.plots().iter().filter(|p| !p.is_empty()).count();
    let blooming = garden
        .plots()
        .iter()
        .filter(|p| p.plant().is_some_and(Plant::is_blooming))
        .count();
    let season_name = season_title(options.season);
    let glyph = compact_glyph(garden, options.sprites);

    let mut lb = LineBuilder::default();
    lb.push_styled(
        format!(" {} {} ", glyph, product::DISPLAY_NAME),
        palette.header,
    );
    lb.push_styled(
        format!("{} {} ", box_chars.sep, season_name),
        palette.ambient,
    );
    if garden.is_dormant() {
        lb.push_styled("[Dormant] ", palette.muted);
    }
    lb.push_styled(
        format!(
            "{} {planted}/{total_plots} planted {} {blooming} blooming",
            box_chars.sep, box_chars.sep
        ),
        palette.foliage,
    );
    if width >= 68 {
        let tag = seasonal_header_tag(options.sprites, options.season, phase);
        let tag_w = display_width(tag);
        if lb.width + tag_w < width {
            lb.push_plain(" ".repeat(width - lb.width - tag_w));
            lb.push_styled(
                tag,
                if garden.is_dormant() {
                    palette.muted
                } else {
                    palette.ambient
                },
            );
        }
    }
    lb.into_fitted(width, true)
}

fn build_footer_line(
    garden: &Garden,
    width: usize,
    options: RenderOptions,
    palette: &SeasonPalette,
    box_chars: &BoxChars,
) -> RenderedLine {
    let glyph = compact_glyph(garden, options.sprites);
    let state_text = if garden.is_dormant() {
        "State: Dormant (resting cozily; activity wakes the garden)"
    } else if width >= 72 {
        "State: Growing quietly (plants rest safely when away)"
    } else {
        "State: Growing quietly"
    };

    let mut lb = LineBuilder::default();
    lb.push_styled(format!(" Glyph {glyph} "), palette.header);
    lb.push_styled(format!("{} {state_text}", box_chars.sep), palette.muted);
    lb.into_fitted(width, true)
}

#[allow(clippy::too_many_arguments)]
fn render_plot_cell(
    plot_index: usize,
    plot: &Plot,
    dormant: bool,
    catalog: &SpeciesCatalog,
    sprites: &SpriteBank,
    cell_w: usize,
    cell_h: usize,
    options: RenderOptions,
    palette: &SeasonPalette,
    box_chars: &BoxChars,
    phase: u8,
) -> Vec<LineBuilder> {
    if cell_h >= 9 {
        render_spacious_cell(
            plot_index, plot, dormant, catalog, sprites, cell_w, cell_h, options, palette,
            box_chars, phase,
        )
    } else if cell_h >= 6 {
        render_standard_cell(
            plot_index, plot, dormant, catalog, sprites, cell_w, cell_h, options, palette,
            box_chars, phase,
        )
    } else if cell_h >= 4 {
        render_medium_cell(
            plot_index, plot, dormant, catalog, sprites, cell_w, cell_h, options, palette,
            box_chars,
        )
    } else {
        render_compact_grid_cell(
            plot_index, plot, dormant, catalog, cell_w, cell_h, options, palette, box_chars,
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn render_compact_grid_cell(
    plot_index: usize,
    plot: &Plot,
    dormant: bool,
    catalog: &SpeciesCatalog,
    cell_w: usize,
    cell_h: usize,
    options: RenderOptions,
    palette: &SeasonPalette,
    box_chars: &BoxChars,
) -> Vec<LineBuilder> {
    let plot_num = plot_index + 1;
    let mut lines = Vec::with_capacity(cell_h);

    if let Some(plant) = plot.plant() {
        let species = catalog.get(&plant.species_id);
        let hint = species.map_or(ColorHint::Green, |sp| sp.color_hint);
        let p_style = plant_style(hint, plant.stage, options.season, dormant);
        let inline = species.map_or_else(
            || default_inline_sprite(options.sprites, plant.stage),
            |sp| sp.inline_sprite(options.sprites, plant.stage),
        );
        let badge = short_stage_tag(plant.stage, box_chars.bloom_mark);
        let name = species.map_or(plant.species_id.as_str(), |sp| sp.display_name.as_str());

        let mut row0 = LineBuilder::default();
        let prefix = format!("#{plot_num} ");
        let sprite_col = pad_center_to_width(inline, 3);
        let used_left = display_width(&prefix) + 3;
        row0.push_styled(prefix, palette.muted);
        row0.push_styled(sprite_col, p_style);
        let rem = cell_w.saturating_sub(used_left);
        if rem > 0 {
            let right = format!("{:>width$}", badge, width = rem);
            row0.push_styled(truncate_to_width(&right, rem), p_style);
        }
        lines.push(row0);

        let mut row1 = LineBuilder::default();
        row1.push_styled(
            pad_right_to_width(name, cell_w),
            if dormant { palette.muted } else { p_style },
        );
        lines.push(row1);
    } else {
        let empty_glyph = match options.sprites {
            SpriteSet::Unicode => " · ",
            SpriteSet::Ascii => " . ",
        };
        let mut row0 = LineBuilder::default();
        row0.push_styled(
            pad_right_to_width(&format!("#{plot_num} {empty_glyph}   --"), cell_w),
            palette.muted,
        );
        lines.push(row0);

        let mut row1 = LineBuilder::default();
        row1.push_styled(pad_right_to_width("(empty)", cell_w), palette.muted);
        lines.push(row1);
    }

    while lines.len() < cell_h {
        lines.push(LineBuilder::default());
    }
    lines
}

#[allow(clippy::too_many_arguments)]
fn render_medium_cell(
    plot_index: usize,
    plot: &Plot,
    dormant: bool,
    catalog: &SpeciesCatalog,
    sprites: &SpriteBank,
    cell_w: usize,
    cell_h: usize,
    options: RenderOptions,
    palette: &SeasonPalette,
    box_chars: &BoxChars,
) -> Vec<LineBuilder> {
    let plot_num = plot_index + 1;
    let mut lines = Vec::with_capacity(cell_h);

    if let Some(plant) = plot.plant() {
        let species = catalog.get(&plant.species_id);
        let hint = species.map_or(ColorHint::Green, |sp| sp.color_hint);
        let p_style = plant_style(hint, plant.stage, options.season, dormant);
        let name = species.map_or(plant.species_id.as_str(), |sp| sp.display_name.as_str());
        let badge = short_stage_tag(plant.stage, box_chars.bloom_mark);
        let right = if dormant {
            if cell_w >= 18 {
                format!(" {badge} [Rest] ")
            } else {
                " [Rest] ".to_string()
            }
        } else {
            format!(" {badge} ")
        };

        lines.push(build_lr_line(
            &format!(" #{plot_num} {name}"),
            &right,
            cell_w,
            if dormant {
                palette.muted
            } else {
                palette.header
            },
            p_style,
        ));
        for row in sprites.art_for(options.sprites, &plant.species_id, plant.stage) {
            let mut lb = LineBuilder::default();
            lb.push_styled(pad_center_to_width(row, cell_w), p_style);
            lines.push(lb);
        }
    } else {
        let mut title = LineBuilder::default();
        title.push_styled(
            pad_right_to_width(&format!(" #{plot_num} (empty)"), cell_w),
            palette.muted,
        );
        lines.push(title);
        for row in empty_plot_art(options.sprites) {
            let mut lb = LineBuilder::default();
            lb.push_styled(pad_center_to_width(row, cell_w), palette.soil);
            lines.push(lb);
        }
    }

    pad_vertical_lines(lines, cell_h)
}

#[allow(clippy::too_many_arguments)]
fn render_standard_sprite_row(
    row_idx: usize,
    sprite_row: &str,
    sprite_style: StyleSpec,
    ambient_style: StyleSpec,
    plot_index: usize,
    cell_w: usize,
    options: RenderOptions,
    phase: u8,
) -> LineBuilder {
    let mut lb = LineBuilder::default();
    if row_idx == 0 && cell_w >= 19 {
        let (left_wing, right_wing) =
            seasonal_cell_motes(options.sprites, options.season, plot_index, phase);
        // 3 + 2 + 7 + 2 + 3 = 17 columns; (cell_w - 17) / 2 + 5 == (cell_w - 7) / 2
        // for every cell_w >= 17, keeping row 0 aligned to the exact column of rows 1..=2.
        let block_w = 17;
        let left_pad = (cell_w - block_w) / 2;
        let right_pad = cell_w - block_w - left_pad;
        lb.push_plain(" ".repeat(left_pad));
        lb.push_styled(left_wing, ambient_style);
        lb.push_plain("  ");
        lb.push_styled(sprite_row, sprite_style);
        lb.push_plain("  ");
        lb.push_styled(right_wing, ambient_style);
        lb.push_plain(" ".repeat(right_pad));
    } else {
        lb.push_styled(pad_center_to_width(sprite_row, cell_w), sprite_style);
    }
    lb
}

#[allow(clippy::too_many_arguments)]
fn render_standard_cell(
    plot_index: usize,
    plot: &Plot,
    dormant: bool,
    catalog: &SpeciesCatalog,
    sprites: &SpriteBank,
    cell_w: usize,
    cell_h: usize,
    options: RenderOptions,
    palette: &SeasonPalette,
    box_chars: &BoxChars,
    phase: u8,
) -> Vec<LineBuilder> {
    let plot_num = plot_index + 1;
    let mut lines = Vec::with_capacity(6);
    let ambient_style = if dormant {
        palette.muted
    } else {
        palette.ambient
    };

    if let Some(plant) = plot.plant() {
        let species = catalog.get(&plant.species_id);
        let hint = species.map_or(ColorHint::Green, |sp| sp.color_hint);
        let rarity = species.map_or(Rarity::Common, |sp| sp.rarity);
        let p_style = plant_style(hint, plant.stage, options.season, dormant);
        let r_style = rarity_style(rarity, options.season);
        let name = species.map_or(plant.species_id.as_str(), |sp| sp.display_name.as_str());

        // Row 0: `#1 Meadow Daisy     [C]`
        lines.push(build_lr_line(
            &format!(" #{plot_num} {name}"),
            &format!(" [{}] ", rarity_short(rarity)),
            cell_w,
            if dormant { palette.muted } else { p_style },
            r_style,
        ));

        // Rows 1..=3: 3x7 multi-line plant sprite centered with subtle seasonal sky motes.
        for (row_idx, row) in sprites
            .art_for(options.sprites, &plant.species_id, plant.stage)
            .into_iter()
            .enumerate()
        {
            lines.push(render_standard_sprite_row(
                row_idx,
                row,
                p_style,
                ambient_style,
                plot_index,
                cell_w,
                options,
                phase,
            ));
        }

        // Row 4: Planter pot rim centered beneath the 7-col sprite.
        let mut pot_lb = LineBuilder::default();
        pot_lb.push_styled(
            pad_center_to_width(pot_rim(options.sprites), cell_w),
            palette.pot,
        );
        lines.push(pot_lb);

        // Row 5: Stage label + stage GP progress bar.
        let stage_label = if dormant {
            format!(" {} [Rest]", stage_title(plant.stage))
        } else if plant.is_blooming() {
            format!(" Blooming {}", box_chars.bloom_mark)
        } else {
            format!(" {}", stage_title(plant.stage))
        };
        let bar = format!("{} ", format_progress_bar(plant, species, 6, box_chars));
        lines.push(build_lr_line(
            &stage_label,
            &bar,
            cell_w,
            p_style,
            if dormant {
                palette.muted
            } else {
                palette.foliage
            },
        ));
    } else {
        let mut title = LineBuilder::default();
        title.push_styled(
            pad_right_to_width(&format!(" #{plot_num} (empty plot)"), cell_w),
            palette.muted,
        );
        lines.push(title);

        for (row_idx, row) in empty_plot_art(options.sprites).into_iter().enumerate() {
            lines.push(render_standard_sprite_row(
                row_idx,
                row,
                palette.soil,
                ambient_style,
                plot_index,
                cell_w,
                options,
                phase,
            ));
        }

        let mut pot_lb = LineBuilder::default();
        pot_lb.push_styled(
            pad_center_to_width(pot_rim(options.sprites), cell_w),
            palette.muted,
        );
        lines.push(pot_lb);

        let mut hint_lb = LineBuilder::default();
        hint_lb.push_styled(pad_center_to_width("ready to plant", cell_w), palette.muted);
        lines.push(hint_lb);
    }

    pad_vertical_lines(lines, cell_h)
}

#[allow(clippy::too_many_arguments)]
fn render_spacious_cell(
    plot_index: usize,
    plot: &Plot,
    dormant: bool,
    catalog: &SpeciesCatalog,
    sprites: &SpriteBank,
    cell_w: usize,
    cell_h: usize,
    options: RenderOptions,
    palette: &SeasonPalette,
    box_chars: &BoxChars,
    phase: u8,
) -> Vec<LineBuilder> {
    let plot_num = plot_index + 1;
    let mut lines = Vec::with_capacity(11);
    let (pot_top, pot_bottom) = wide_pot(options.sprites);
    let ambient_style = if dormant {
        palette.muted
    } else {
        palette.ambient
    };

    if let Some(plant) = plot.plant() {
        let species = catalog.get(&plant.species_id);
        let hint = species.map_or(ColorHint::Green, |sp| sp.color_hint);
        let rarity = species.map_or(Rarity::Common, |sp| sp.rarity);
        let p_style = plant_style(hint, plant.stage, options.season, dormant);
        let r_style = rarity_style(rarity, options.season);
        let name = species.map_or(plant.species_id.as_str(), |sp| sp.display_name.as_str());

        // Line 0: `#1 · Sakura Bonsai            mythic `
        lines.push(build_lr_line(
            &format!(" #{plot_num} {} {name}", box_chars.sep),
            &format!(" {} ", rarity.label()),
            cell_w,
            if dormant { palette.muted } else { p_style },
            r_style,
        ));

        // Line 1: Cozy status subtitle.
        let status_note = if dormant {
            format!("   {} Resting cozily (dormant)", box_chars.sep)
        } else if plant.is_blooming() {
            format!(
                "   {} In full bloom {} ready to harvest",
                box_chars.sep, box_chars.bloom_mark
            )
        } else {
            format!("   {} Growing steadily", box_chars.sep)
        };
        let mut sub_lb = LineBuilder::default();
        sub_lb.push_styled(
            pad_right_to_width(&status_note, cell_w),
            if plant.is_blooming() && !dormant {
                p_style
            } else {
                palette.muted
            },
        );
        lines.push(sub_lb);

        // Line 2: Seasonal canopy sky sprite (`petals`, `fireflies`, `leaves`, `snow`).
        if cell_h >= 10 {
            let mut aura_lb = LineBuilder::default();
            let aura = seasonal_canopy_aura(
                options.sprites,
                options.season,
                plant.is_blooming(),
                plot_index,
                phase,
            );
            aura_lb.push_styled(pad_center_to_width(aura, cell_w), ambient_style);
            lines.push(aura_lb);
        }

        // Lines 3..=5: 3x7 multi-line plant sprite centered.
        for row in sprites.art_for(options.sprites, &plant.species_id, plant.stage) {
            debug_assert_eq!(display_width(row), SPRITE_ART_WIDTH);
            let mut lb = LineBuilder::default();
            lb.push_styled(pad_center_to_width(row, cell_w), p_style);
            lines.push(lb);
        }

        // Lines 6..=7: Planter pot top & base.
        let mut pt_lb = LineBuilder::default();
        pt_lb.push_styled(pad_center_to_width(pot_top, cell_w), palette.pot);
        lines.push(pt_lb);

        let mut pb_lb = LineBuilder::default();
        pb_lb.push_styled(pad_center_to_width(pot_bottom, cell_w), palette.pot);
        lines.push(pb_lb);

        // Line 8: Explicit stage title.
        let stage_line = if plant.is_blooming() {
            format!(" Stage: Blooming {}", box_chars.bloom_mark)
        } else {
            format!(" Stage: {}", stage_title(plant.stage))
        };
        let pct_str = format!("{} ", format_stage_percent(plant, species));
        lines.push(build_lr_line(
            &stage_line,
            &pct_str,
            cell_w,
            p_style,
            palette.muted,
        ));

        // Line 9: Wide stage GP progress bar.
        let bar_slots = cell_w.saturating_sub(14).clamp(6, 18);
        let bar = format_progress_bar(plant, species, bar_slots, box_chars);
        let mut bar_lb = LineBuilder::default();
        bar_lb.push_styled(" Progress ", palette.muted);
        bar_lb.push_styled(
            bar,
            if dormant {
                palette.muted
            } else {
                palette.foliage
            },
        );
        lines.push(bar_lb);

        // Line 10 (when cell_h >= 11): Preferred seasons line.
        if cell_h >= 11 {
            let seasons_csv = species.map_or_else(
                || "any".to_string(),
                |sp| {
                    sp.preferred_seasons
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                },
            );
            let mut pref_lb = LineBuilder::default();
            pref_lb.push_styled(
                pad_right_to_width(&format!(" Season: {seasons_csv}"), cell_w),
                palette.muted,
            );
            lines.push(pref_lb);
        }
    } else {
        lines.push(build_lr_line(
            &format!(" #{plot_num} {} Empty Plot", box_chars.sep),
            "-- ",
            cell_w,
            palette.muted,
            palette.muted,
        ));

        let mut sub_lb = LineBuilder::default();
        sub_lb.push_styled(
            pad_right_to_width(&format!("   {} Soil rested & ready", box_chars.sep), cell_w),
            palette.muted,
        );
        lines.push(sub_lb);

        if cell_h >= 10 {
            let mut aura_lb = LineBuilder::default();
            let aura =
                seasonal_canopy_aura(options.sprites, options.season, false, plot_index, phase);
            aura_lb.push_styled(pad_center_to_width(aura, cell_w), ambient_style);
            lines.push(aura_lb);
        }

        for row in empty_plot_art(options.sprites) {
            let mut lb = LineBuilder::default();
            lb.push_styled(pad_center_to_width(row, cell_w), palette.soil);
            lines.push(lb);
        }

        let mut pt_lb = LineBuilder::default();
        pt_lb.push_styled(pad_center_to_width(pot_top, cell_w), palette.muted);
        lines.push(pt_lb);

        let mut pb_lb = LineBuilder::default();
        pb_lb.push_styled(pad_center_to_width(pot_bottom, cell_w), palette.muted);
        lines.push(pb_lb);

        let mut st_lb = LineBuilder::default();
        st_lb.push_styled(
            pad_right_to_width(" Stage: (unplanted)", cell_w),
            palette.muted,
        );
        lines.push(st_lb);

        let bar_slots = cell_w.saturating_sub(14).clamp(6, 18);
        let empty_bar = format!(
            "[{}]",
            std::iter::repeat_n(box_chars.bar_empty, bar_slots).collect::<String>()
        );
        let mut bar_lb = LineBuilder::default();
        bar_lb.push_styled(
            pad_right_to_width(&format!(" Progress {empty_bar}"), cell_w),
            palette.muted,
        );
        lines.push(bar_lb);

        if cell_h >= 11 {
            let mut hint_lb = LineBuilder::default();
            hint_lb.push_styled(
                pad_right_to_width(" Plant a seed whenever you like", cell_w),
                palette.muted,
            );
            lines.push(hint_lb);
        }
    }

    pad_vertical_lines(lines, cell_h)
}

fn render_compact_list(
    garden: &Garden,
    catalog: &SpeciesCatalog,
    width: usize,
    height: usize,
    options: RenderOptions,
) -> Vec<RenderedLine> {
    let palette = SeasonPalette::for_season(options.season);
    let box_chars = BoxChars::for_set(options.sprites);
    let glyph = compact_glyph(garden, options.sprites);
    let mut out = Vec::with_capacity(height.min(garden.plots().len() + 1));

    // Row 0: concise header line.
    let mut header = LineBuilder::default();
    header.push_styled(
        format!("{glyph} {} ", product::DISPLAY_NAME),
        palette.header,
    );
    header.push_styled(
        format!("{} {}", box_chars.sep, season_title(options.season)),
        palette.ambient,
    );
    if garden.is_dormant() {
        header.push_styled(" [Dormant]", palette.muted);
    }
    out.push(header.into_fitted(width, false));

    if height <= 1 {
        return out;
    }

    let plots = garden.plots();
    let avail_rows = height - 1;
    let show_overflow_footer = plots.len() > avail_rows && avail_rows >= 2;
    let plot_rows = if show_overflow_footer {
        avail_rows - 1
    } else {
        avail_rows.min(plots.len())
    };

    for (idx, plot) in plots.iter().enumerate().take(plot_rows) {
        let plot_num = idx + 1;
        let mut lb = LineBuilder::default();
        if let Some(plant) = plot.plant() {
            let species = catalog.get(&plant.species_id);
            let hint = species.map_or(ColorHint::Green, |sp| sp.color_hint);
            let p_style = plant_style(hint, plant.stage, options.season, garden.is_dormant());
            let inline = species.map_or_else(
                || default_inline_sprite(options.sprites, plant.stage),
                |sp| sp.inline_sprite(options.sprites, plant.stage),
            );
            let name = species.map_or(plant.species_id.as_str(), |sp| sp.display_name.as_str());
            let badge = short_stage_tag(plant.stage, box_chars.bloom_mark);

            lb.push_styled(format!("#{plot_num} "), palette.muted);
            lb.push_styled(format!("{} ", pad_center_to_width(inline, 3)), p_style);
            lb.push_styled(format!("[{badge}] "), p_style);
            lb.push_styled(
                name,
                if garden.is_dormant() {
                    palette.muted
                } else {
                    p_style
                },
            );
        } else {
            let empty_dot = match options.sprites {
                SpriteSet::Unicode => " · ",
                SpriteSet::Ascii => " . ",
            };
            lb.push_styled(format!("#{plot_num} {empty_dot} (empty)"), palette.muted);
        }
        out.push(lb.into_fitted(width, false));
    }

    if show_overflow_footer {
        let remaining = plots.len() - plot_rows;
        let mut more = LineBuilder::default();
        more.push_styled(format!("... +{remaining} more plots"), palette.muted);
        out.push(more.into_fitted(width, false));
    }

    out
}

fn build_lr_line(
    left: &str,
    right: &str,
    target_width: usize,
    left_style: StyleSpec,
    right_style: StyleSpec,
) -> LineBuilder {
    let mut lb = LineBuilder::default();
    let rw = display_width(right);
    if rw >= target_width {
        lb.push_styled(pad_right_to_width(left, target_width), left_style);
        return lb;
    }
    let left_budget = target_width - rw;
    let fitted_left = pad_right_to_width(left, left_budget);
    lb.push_styled(fitted_left, left_style);
    lb.push_styled(right, right_style);
    lb
}

fn pad_vertical_lines(mut content: Vec<LineBuilder>, target_h: usize) -> Vec<LineBuilder> {
    if content.len() >= target_h {
        content.truncate(target_h);
        return content;
    }
    let extra = target_h - content.len();
    let top_pad = extra / 2;
    let bottom_pad = extra - top_pad;
    let mut out = Vec::with_capacity(target_h);
    for _ in 0..top_pad {
        out.push(LineBuilder::default());
    }
    out.append(&mut content);
    for _ in 0..bottom_pad {
        out.push(LineBuilder::default());
    }
    out
}

fn format_progress_bar(
    plant: &Plant,
    species: Option<&Species>,
    slots: usize,
    box_chars: &BoxChars,
) -> String {
    let filled = match plant.stage.transition_index() {
        None => slots,
        Some(idx) => {
            let threshold = species
                .map_or(100_000_u64, |sp| sp.gp_per_stage[idx])
                .max(1);
            let clamped = plant.stage_gp.min(threshold);
            let num = u128::from(clamped) * (slots as u128);
            usize::try_from(num / u128::from(threshold)).unwrap_or(slots)
        }
    }
    .min(slots);

    let mut out = String::with_capacity(slots * 3 + 2);
    out.push('[');
    out.extend(std::iter::repeat_n(box_chars.bar_fill, filled));
    out.extend(std::iter::repeat_n(box_chars.bar_empty, slots - filled));
    out.push(']');
    out
}

fn format_stage_percent(plant: &Plant, species: Option<&Species>) -> String {
    match plant.stage.transition_index() {
        None => "100%".to_string(),
        Some(idx) => {
            let threshold = species
                .map_or(100_000_u64, |sp| sp.gp_per_stage[idx])
                .max(1);
            let pct = (u128::from(plant.stage_gp.min(threshold)) * 100) / u128::from(threshold);
            format!("{pct}%")
        }
    }
}

const fn season_title(season: Season) -> &'static str {
    match season {
        Season::Spring => "Spring",
        Season::Summer => "Summer",
        Season::Autumn => "Autumn",
        Season::Winter => "Winter",
    }
}

const fn stage_title(stage: Stage) -> &'static str {
    match stage {
        Stage::Seed => "Seed",
        Stage::Sprout => "Sprout",
        Stage::Young => "Young",
        Stage::Mature => "Mature",
        Stage::Blooming => "Blooming",
    }
}

fn short_stage_tag(stage: Stage, bloom_mark: &str) -> String {
    match stage {
        Stage::Seed => "Seed".to_string(),
        Stage::Sprout => "Sprt".to_string(),
        Stage::Young => "Yng".to_string(),
        Stage::Mature => "Matr".to_string(),
        Stage::Blooming => format!("Blm{bloom_mark}"),
    }
}

const fn rarity_short(rarity: Rarity) -> &'static str {
    match rarity {
        Rarity::Common => "C",
        Rarity::Uncommon => "U",
        Rarity::Rare => "R",
        Rarity::Mythic => "M",
    }
}

const fn default_inline_sprite(set: SpriteSet, stage: Stage) -> &'static str {
    match (set, stage) {
        (SpriteSet::Unicode, Stage::Seed) => " · ",
        (SpriteSet::Unicode, Stage::Sprout) => " ╷ ",
        (SpriteSet::Unicode, Stage::Young) => " │ ",
        (SpriteSet::Unicode, Stage::Mature) => "├│┤",
        (SpriteSet::Unicode, Stage::Blooming) => "░◈░",
        (SpriteSet::Ascii, Stage::Seed) => " . ",
        (SpriteSet::Ascii, Stage::Sprout) => " , ",
        (SpriteSet::Ascii, Stage::Young) => " | ",
        (SpriteSet::Ascii, Stage::Mature) => "-|-",
        (SpriteSet::Ascii, Stage::Blooming) => "*@*",
    }
}
