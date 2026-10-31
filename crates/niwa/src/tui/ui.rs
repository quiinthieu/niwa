//! `ratatui` UI views (garden grid, inspector footer, Seed Pouch picker, Almanac, Compost confirm,
//! Away Summary, Help) and `crossterm` interactive terminal loop (PLAN §4.2, §4.3, §7 & §10 T08).

use crate::runtime::GardenSession;
use crate::tui::app::{Overlay, TuiApp};
use crate::tui::scheduler::SIM_TICK_INTERVAL;
use crate::tui::settings::{SettingsField, SettingsState};
use crossterm::event::{self, DisableFocusChange, EnableFocusChange};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::{Frame, Terminal};
use std::io;
use std::time::{Duration, Instant};
use tg_core::catch_up::AwaySummary;
use tg_core::clock::{Clock, Timestamp, TICK_SECS};
use tg_core::model::Stage;
use tg_core::species::{Species, SpriteSet};
use tg_render::color::Ansi16Color;
use tg_render::{ColorTier, RenderedLine, StyleSpec, Viewport};
use tg_signals::consent::ConsentCatalog;
use tg_signals::SignalKind;

/// Draws the complete `TuiApp` view (garden canvas, non-color-only plot inspector bar, status bar,
/// and any active modal overlay) into `frame`.
pub fn draw_ui(frame: &mut Frame<'_>, app: &TuiApp) {
    let area = frame.area();
    if area.width == 0 || area.height == 0 {
        return;
    }

    let (canvas_area, inspector_area, status_area) = split_main_areas(area);

    if canvas_area.height > 0 {
        let rendered = app.session().render_frame_with_phase(
            Viewport::new(canvas_area.width, canvas_area.height),
            None,
            app.color_tier(),
            app.ambient_phase(),
        );
        let selected_num = app.selected_plot() + 1;
        let move_num = app.move_source().map(|idx| idx + 1);
        let lines: Vec<Line<'static>> = rendered
            .lines()
            .iter()
            .map(|line| rendered_line_to_ratatui(line, app.color_tier(), selected_num, move_num))
            .collect();
        frame.render_widget(Paragraph::new(Text::from(lines)), canvas_area);
    }

    if let Some(insp) = inspector_area {
        let inspector_line = build_inspector_line(app);
        frame.render_widget(Paragraph::new(inspector_line), insp);
    }

    if let Some(stat) = status_area {
        let status_style = if app.color_tier().is_colored() {
            Style::default().add_modifier(Modifier::DIM)
        } else {
            Style::default()
        };
        let status_text =
            sanitize_for_sprites(app.status_message(), app.session().config().sprites);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(status_text, status_style))),
            stat,
        );
    }

    if let Some(overlay) = app.overlay() {
        match overlay {
            Overlay::AwaySummary(summary) => draw_away_modal(frame, area, app, summary),
            Overlay::SeedPicker { cursor } => draw_seed_picker_modal(frame, area, app, *cursor),
            Overlay::ConfirmCompost { plot_index } => {
                draw_compost_modal(frame, area, app, *plot_index);
            }
            Overlay::Almanac { cursor } => draw_almanac_modal(frame, area, app, *cursor),
            Overlay::Settings(state) => draw_settings_modal(frame, area, app, state),
            Overlay::SignalConsent { signal, .. } => {
                draw_signal_consent_modal(frame, area, app, *signal);
            }
            Overlay::Help => draw_help_modal(frame, area, app),
        }
    }
}

/// Runs the interactive `crossterm` + `ratatui` terminal loop until the user quits (`q` / `Ctrl-C`).
pub fn run_interactive_tui(
    session: GardenSession,
    color_tier: ColorTier,
    clock: &dyn Clock,
) -> anyhow::Result<()> {
    let mut app = TuiApp::new(session, color_tier, clock)?;

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableFocusChange)?;
    let _restore_guard = TerminalRestoreGuard;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    // Monotonic origin used only to enforce the §7 frame-rate cap (`<= 4 fps`) and `30 s` idle cutoff.
    let start = Instant::now();
    app.step(None, Duration::ZERO, clock, &mut terminal)?;

    while !app.should_quit() {
        let mono_now = start.elapsed();
        // Power budget (PLAN §2 & §7): compute remaining wall seconds until the next 60 s garden tick
        // so an idle (`0 fps`) TUI sleeps directly until the next 60 s tick with zero animation wakes.
        let elapsed_since_tick = u64::try_from(
            clock
                .now()
                .as_unix_secs()
                .saturating_sub(app.session().garden().last_tick_at().as_unix_secs())
                .max(0),
        )
        .unwrap_or(0);
        let tick_period = TICK_SECS as u64;
        let next_tick_in = if elapsed_since_tick >= tick_period {
            Duration::ZERO
        } else {
            SIM_TICK_INTERVAL.saturating_sub(Duration::from_secs(elapsed_since_tick))
        };

        // Power budget (PLAN §4.3 & §7): `next_poll_timeout` sleeps `>= 250 ms` between frames while
        // focused/active (`<= 4 fps`) and up to `60 s` (`next_tick_in`) when idle or unfocused (`0 fps`).
        let poll_timeout = app.scheduler().next_poll_timeout(mono_now, next_tick_in);
        let maybe_event = if event::poll(poll_timeout)? {
            Some(event::read()?)
        } else {
            None
        };

        app.step(maybe_event, start.elapsed(), clock, &mut terminal)?;
    }

    terminal.show_cursor()?;
    Ok(())
}

struct TerminalRestoreGuard;

impl Drop for TerminalRestoreGuard {
    fn drop(&mut self) {
        let mut stdout = io::stdout();
        let _ = execute!(stdout, DisableFocusChange, LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

fn split_main_areas(area: Rect) -> (Rect, Option<Rect>, Option<Rect>) {
    if area.height >= 5 {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(3),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(area);
        (chunks[0], Some(chunks[1]), Some(chunks[2]))
    } else if area.height >= 2 {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(area);
        (chunks[0], Some(chunks[1]), None)
    } else {
        (area, None, None)
    }
}

const ASCII_MODAL_BORDERS: ratatui::symbols::border::Set<'static> = ratatui::symbols::border::Set {
    top_left: "+",
    top_right: "+",
    bottom_left: "+",
    bottom_right: "+",
    vertical_left: "|",
    vertical_right: "|",
    horizontal_top: "-",
    horizontal_bottom: "-",
};

fn sanitize_for_sprites(text: &str, sprites: SpriteSet) -> String {
    if sprites != SpriteSet::Ascii
        || text
            .bytes()
            .all(|b| b == b'\n' || (0x20..=0x7E).contains(&b))
    {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\n' | ' '..='~' => out.push(ch),
            '—' | '–' | '·' => out.push('-'),
            '•' => out.push('*'),
            '…' => out.push_str("..."),
            '§' => out.push_str("sec."),
            _ => out.push('?'),
        }
    }
    out
}

fn rendered_line_to_ratatui(
    line: &RenderedLine,
    tier: ColorTier,
    selected_num: usize,
    move_num: Option<usize>,
) -> Line<'static> {
    let selected_token = format!(" #{selected_num} ");
    let selected_replacement = format!(">#{selected_num} ");
    let compact_selected_token = format!("#{selected_num} ");
    let compact_selected_replacement = format!(">#{selected_num}");
    let move_pair = move_num.map(|n| {
        (
            format!(" #{n} "),
            format!("*#{n} "),
            format!("#{n} "),
            format!("*#{n}"),
        )
    });

    let mut spans = Vec::with_capacity(line.spans().len());
    for span in line.spans() {
        let mut text = span.text.clone();
        let mut is_selected_header = false;
        if text.contains(&selected_token) {
            text = text.replacen(&selected_token, &selected_replacement, 1);
            is_selected_header = true;
        } else if text.starts_with(&compact_selected_token) {
            text = text.replacen(&compact_selected_token, &compact_selected_replacement, 1);
            is_selected_header = true;
        } else if let Some((ref mv_tok, ref mv_rep, ref mv_compact_tok, ref mv_compact_rep)) =
            move_pair
        {
            if text.contains(mv_tok) {
                text = text.replacen(mv_tok, mv_rep, 1);
            } else if text.starts_with(mv_compact_tok) {
                text = text.replacen(mv_compact_tok, mv_compact_rep, 1);
            }
        }

        let mut style = span
            .style
            .map_or_else(Style::default, |spec| style_spec_to_ratatui(spec, tier));
        if is_selected_header {
            style = style.add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
        }
        spans.push(Span::styled(text, style));
    }
    Line::from(spans)
}

fn style_spec_to_ratatui(spec: StyleSpec, tier: ColorTier) -> Style {
    let mut style = Style::default();
    if spec.bold {
        style = style.add_modifier(Modifier::BOLD);
    }
    if spec.dim {
        style = style.add_modifier(Modifier::DIM);
    }

    match tier {
        ColorTier::NoColor => style,
        ColorTier::Ansi16 => style.fg(ansi16_to_ratatui(spec.ansi16)),
        ColorTier::Ansi256 => style.fg(Color::Indexed(spec.ansi256)),
        ColorTier::Truecolor => {
            let (r, g, b) = spec.rgb;
            style.fg(Color::Rgb(r, g, b))
        }
    }
}

const fn ansi16_to_ratatui(c: Ansi16Color) -> Color {
    match c {
        Ansi16Color::Black => Color::Black,
        Ansi16Color::Red => Color::Red,
        Ansi16Color::Green => Color::Green,
        Ansi16Color::Yellow => Color::Yellow,
        Ansi16Color::Blue => Color::Blue,
        Ansi16Color::Magenta => Color::Magenta,
        Ansi16Color::Cyan => Color::Cyan,
        Ansi16Color::White => Color::Gray,
        Ansi16Color::BrightBlack => Color::DarkGray,
        Ansi16Color::BrightRed => Color::LightRed,
        Ansi16Color::BrightGreen => Color::LightGreen,
        Ansi16Color::BrightYellow => Color::LightYellow,
        Ansi16Color::BrightBlue => Color::LightBlue,
        Ansi16Color::BrightMagenta => Color::LightMagenta,
        Ansi16Color::BrightCyan => Color::LightCyan,
        Ansi16Color::BrightWhite => Color::White,
    }
}

fn build_inspector_line(app: &TuiApp) -> Line<'static> {
    let garden = app.session().garden();
    let sprites = app.session().config().sprites;
    let sep = match sprites {
        SpriteSet::Unicode => "·",
        SpriteSet::Ascii => "-",
    };
    let total_plots = garden.plots().len();
    let idx = app.selected_plot();
    let plot_num = idx + 1;
    let total_seeds: u32 = app.session().save().inventory.values().copied().sum();
    let current_season = app.session().current_season();

    let plot_desc = match garden.plots().get(idx).and_then(|p| p.plant()) {
        Some(plant) => {
            let sp_opt = app.session().catalog().get(&plant.species_id);
            let (name, rarity) = sp_opt.map_or_else(
                || (plant.species_id.as_str(), "common"),
                |sp| (sp.display_name.as_str(), sp.rarity.label()),
            );
            let stage_str = match plant.stage {
                Stage::Seed => "Seed",
                Stage::Sprout => "Sprout",
                Stage::Young => "Young",
                Stage::Mature => "Mature",
                Stage::Blooming => "Blooming *",
            };
            let season_wait = match (plant.stage, sp_opt) {
                (Stage::Seed, Some(sp))
                    if sp.seasonal_sprout_only && !sp.can_sprout_in(current_season) =>
                {
                    let sprout_seasons: Vec<&str> =
                        sp.preferred_seasons.iter().map(|s| s.as_str()).collect();
                    format!(" (awaits {})", sprout_seasons.join("/"))
                }
                _ => String::new(),
            };
            let rest_tag = if garden.is_dormant() {
                " [Resting]"
            } else {
                ""
            };
            format!("{name} [{rarity}] {sep} {stage_str}{season_wait}{rest_tag}")
        }
        None => "Empty - press [p] to plant".to_string(),
    };

    let move_tag = app
        .move_source()
        .map_or_else(String::new, |src| format!(" [Moving Plot {}]", src + 1));

    let text = format!(
        "> Plot {plot_num}/{total_plots}: {plot_desc}{move_tag} | Pouch: {total_seeds} seeds"
    );
    Line::from(Span::styled(
        sanitize_for_sprites(&text, sprites),
        Style::default().add_modifier(Modifier::BOLD),
    ))
}

fn modal_block(title: String, sprites: SpriteSet) -> Block<'static> {
    let borders = Block::default().borders(Borders::ALL).title(title);
    if sprites == SpriteSet::Ascii {
        borders.border_set(ASCII_MODAL_BORDERS)
    } else {
        borders.border_type(ratatui::widgets::BorderType::Rounded)
    }
}

fn centered_rect(area: Rect, max_width: u16, max_height: u16) -> Rect {
    let width = max_width
        .min(area.width.saturating_sub(2))
        .max(10)
        .min(area.width);
    let height = max_height
        .min(area.height.saturating_sub(2))
        .max(5)
        .min(area.height);
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect::new(x, y, width, height)
}

fn draw_away_modal(frame: &mut Frame<'_>, area: Rect, app: &TuiApp, summary: &AwaySummary) {
    let rect = centered_rect(area, 64, 14);
    frame.render_widget(Clear, rect);
    let sprites = app.session().config().sprites;
    let bullet_mark = match sprites {
        SpriteSet::Unicode => "•",
        SpriteSet::Ascii => "*",
    };

    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled(
        sanitize_for_sprites(&summary.headline, sprites),
        Style::default().add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));
    for bullet in &summary.lines {
        let cleaned = sanitize_for_sprites(bullet, sprites);
        lines.push(Line::from(format!("{bullet_mark} {cleaned}")));
    }
    lines.push(Line::from(""));
    lines.push(Line::from("Press [Enter] or [Esc] to tend your garden."));

    let widget = Paragraph::new(Text::from(lines))
        .block(modal_block(" While you were away ".to_string(), sprites))
        .wrap(Wrap { trim: false });
    frame.render_widget(widget, rect);
}

fn draw_seed_picker_modal(frame: &mut Frame<'_>, area: Rect, app: &TuiApp, cursor: usize) {
    let choices = app.available_seeds();
    let rect = centered_rect(area, 60, 14);
    frame.render_widget(Clear, rect);

    let mut lines = Vec::new();
    lines.push(Line::from(format!(
        "Choose a seed for Plot {} ([Up/Down] select, [Enter] plant, [Esc] cancel):",
        app.selected_plot() + 1
    )));
    lines.push(Line::from(""));

    for (idx, choice) in choices.iter().enumerate() {
        let marker = if idx == cursor { ">" } else { " " };
        let season_note = if choice.can_sprout_now {
            ""
        } else {
            " (rests until season)"
        };
        let row = format!(
            "{marker} {} [{}] x{}{season_note}",
            choice.display_name,
            choice.rarity.label(),
            choice.count
        );
        let style = if idx == cursor {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(row, style)));
    }

    let widget = Paragraph::new(Text::from(lines)).block(modal_block(
        " Seed Pouch ".to_string(),
        app.session().config().sprites,
    ));
    frame.render_widget(widget, rect);
}

fn draw_compost_modal(frame: &mut Frame<'_>, area: Rect, app: &TuiApp, plot_index: usize) {
    let rect = centered_rect(area, 58, 9);
    frame.render_widget(Clear, rect);

    let (name, rarity_word) = app
        .session()
        .garden()
        .plots()
        .get(plot_index)
        .and_then(|p| p.plant())
        .and_then(|plant| app.session().catalog().get(&plant.species_id))
        .map_or(("this plant", "common"), |sp| {
            (sp.display_name.as_str(), sp.rarity.label())
        });

    let lines = vec![
        Line::from(format!("Compost {name} in Plot {}?", plot_index + 1)),
        Line::from(format!("Returns 1 {rarity_word} seed to your seed pouch.")),
        Line::from(""),
        Line::from("[y / Enter] Confirm compost   [n / Esc] Cancel"),
    ];

    let widget = Paragraph::new(Text::from(lines))
        .block(modal_block(
            " Compost Plant ".to_string(),
            app.session().config().sprites,
        ))
        .wrap(Wrap { trim: false });
    frame.render_widget(widget, rect);
}

fn draw_almanac_modal(frame: &mut Frame<'_>, area: Rect, app: &TuiApp, cursor: usize) {
    let rect = centered_rect(area, 76, 22);
    frame.render_widget(Clear, rect);

    let catalog_species: Vec<&Species> = app.session().catalog().iter().collect();
    let almanac = &app.session().save().almanac;
    let discovered_count = almanac.len();
    let total_count = catalog_species.len();

    let inner_height = usize::from(rect.height.saturating_sub(2));
    let list_budget = inner_height.saturating_sub(8).max(3);
    let clamped_cursor = cursor.min(total_count.saturating_sub(1));
    let start_idx = if clamped_cursor >= list_budget {
        clamped_cursor + 1 - list_budget
    } else {
        0
    };
    let end_idx = (start_idx + list_budget).min(total_count);

    let mut lines = Vec::new();
    lines.push(Line::from(format!(
        "Discovered: {discovered_count}/{total_count} species  ([Up/Down] browse, [Esc] close)"
    )));
    lines.push(Line::from(""));

    let sprites = app.session().config().sprites;
    let dash = match sprites {
        SpriteSet::Unicode => "—",
        SpriteSet::Ascii => "-",
    };

    for (idx, sp) in catalog_species
        .iter()
        .enumerate()
        .take(end_idx)
        .skip(start_idx)
    {
        let marker = if idx == clamped_cursor { ">" } else { " " };
        let row_text = if let Some(entry) = almanac.get(&sp.id) {
            let bloom_str = entry.first_bloomed_at.map_or_else(
                || "growing".to_string(),
                |ts| format!("bloomed {}", format_utc_ymd(ts)),
            );
            format!(
                "{marker} {} [{}] {dash} {bloom_str} (harvested {})",
                sp.display_name,
                sp.rarity.label(),
                entry.harvest_count
            )
        } else {
            format!("{marker} ??? [{}] {dash} undiscovered", sp.rarity.label())
        };
        let style = if idx == clamped_cursor {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(row_text, style)));
    }

    lines.push(Line::from(""));
    if let Some(selected_sp) = catalog_species.get(clamped_cursor) {
        if almanac.contains_key(&selected_sp.id) {
            let seasons: Vec<&str> = selected_sp
                .preferred_seasons
                .iter()
                .map(|s| s.as_str())
                .collect();
            let season_line = if selected_sp.seasonal_sprout_only {
                format!("Sprouts in: {} only", seasons.join(", "))
            } else {
                format!("Thrives in: {} (sprouts year-round)", seasons.join(", "))
            };
            lines.push(Line::from(season_line));
            lines.push(Line::from(selected_sp.almanac_flavour.clone()));
        } else {
            lines.push(Line::from(
                "Harvest blooming plants to discover new seasonal seeds.",
            ));
        }
    }

    let widget = Paragraph::new(Text::from(lines))
        .block(modal_block(
            " Almanac ".to_string(),
            app.session().config().sprites,
        ))
        .wrap(Wrap { trim: false });
    frame.render_widget(widget, rect);
}

fn draw_settings_modal(frame: &mut Frame<'_>, area: Rect, app: &TuiApp, state: &SettingsState) {
    let rect = centered_rect(area, 86, 25);
    frame.render_widget(Clear, rect);

    let config = app.session().config();
    let total = SettingsField::ALL.len();
    let clamped_cursor = state.cursor.min(total - 1);

    let inner_height = usize::from(rect.height.saturating_sub(2));
    let list_budget = inner_height.saturating_sub(5).max(3);
    let start_idx = if clamped_cursor >= list_budget {
        clamped_cursor + 1 - list_budget
    } else {
        0
    };
    let end_idx = (start_idx + list_budget).min(total);

    let mut lines = Vec::new();
    lines.push(Line::from(format!(
        "Settings ({}/{} - [Up/Down] select, [Left/Right/Enter] change, [Esc] close)",
        clamped_cursor + 1,
        total
    )));
    lines.push(Line::from(""));

    for (idx, &field) in SettingsField::ALL
        .iter()
        .enumerate()
        .take(end_idx)
        .skip(start_idx)
    {
        let selected = idx == clamped_cursor;
        let marker = if selected { ">" } else { " " };
        let value_text = if selected {
            if let Some(ref buf) = state.edit_buffer {
                format!("[editing: {buf}_]")
            } else {
                field.display_value(config)
            }
        } else {
            field.display_value(config)
        };

        let row = format!(
            "{marker} {:<24} ({:<29}) = {value_text}",
            field.label(),
            field.key()
        );
        let style = if selected {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(row, style)));
    }

    lines.push(Line::from(""));
    let selected_field = SettingsField::ALL[clamped_cursor];
    if let Some(ref err) = state.error_message {
        let cleaned_err = sanitize_for_sprites(err, config.sprites);
        lines.push(Line::from(Span::styled(
            format!("! {cleaned_err} ([Enter] retry, [Ctrl-U] clear, [Esc] cancel)"),
            Style::default().add_modifier(Modifier::BOLD),
        )));
    } else if state.edit_buffer.is_some() {
        lines.push(Line::from(
            "Editing value - [Enter] save via `niwa config set`, [Ctrl-U] clear, [Esc] cancel",
        ));
    } else {
        lines.push(Line::from(selected_field.hint()));
    }

    let widget = Paragraph::new(Text::from(lines))
        .block(modal_block(
            " Settings (config.toml) ".to_string(),
            config.sprites,
        ))
        .wrap(Wrap { trim: false });
    frame.render_widget(widget, rect);
}

fn draw_signal_consent_modal(frame: &mut Frame<'_>, area: Rect, app: &TuiApp, signal: SignalKind) {
    let rect = centered_rect(area, 108, 24);
    frame.render_widget(Clear, rect);

    let catalog = ConsentCatalog::load_default();
    let disclosure = catalog.signal(signal);
    let body = disclosure.render_disclosure_body(signal);

    let mut lines: Vec<Line<'static>> = body
        .lines()
        .enumerate()
        .map(|(idx, line)| {
            if idx == 0 {
                Line::from(Span::styled(
                    line.to_string(),
                    Style::default().add_modifier(Modifier::BOLD),
                ))
            } else {
                Line::from(line.to_string())
            }
        })
        .collect();
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "[y / Enter] Enable signal (records consent)   [n / Esc] Cancel",
        Style::default().add_modifier(Modifier::BOLD),
    )));

    let widget = Paragraph::new(Text::from(lines))
        .block(modal_block(
            format!(" Signal Consent: {} ", signal.as_str()),
            app.session().config().sprites,
        ))
        .wrap(Wrap { trim: false });
    frame.render_widget(widget, rect);
}

fn draw_help_modal(frame: &mut Frame<'_>, area: Rect, app: &TuiApp) {
    let rect = centered_rect(area, 64, 17);
    frame.render_widget(Clear, rect);

    let lines = vec![
        Line::from(Span::styled(
            "Controls",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from("  Arrows / Tab / 1-9   Select garden plot"),
        Line::from("  p / Enter            Plant a seed from your pouch"),
        Line::from("  h / Enter            Harvest a blooming plant for seeds"),
        Line::from("  m                    Move a plant to another empty plot"),
        Line::from("  c                    Compost a plant (returns 1 same-rarity seed)"),
        Line::from("  a                    Open the Almanac"),
        Line::from("  s                    Open Settings (config.toml & signals)"),
        Line::from("  ?                    Toggle this Help screen"),
        Line::from("  q / Ctrl-C           Save and quit"),
        Line::from(""),
        Line::from("Cozy rules: plants grow with time (and sun while open)."),
        Line::from("Nothing you press makes a plant grow, and plants never die."),
    ];

    let widget = Paragraph::new(Text::from(lines))
        .block(modal_block(
            " Help ".to_string(),
            app.session().config().sprites,
        ))
        .wrap(Wrap { trim: false });
    frame.render_widget(widget, rect);
}

/// Converts a Unix [`Timestamp`] into a UTC `YYYY-MM-DD` calendar string using Howard Hinnant's
/// civil-from-days integer algorithm.
#[must_use]
pub fn format_utc_ymd(ts: Timestamp) -> String {
    let z = ts.as_unix_secs().div_euclid(86_400) + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    format!("{year:04}-{m:02}-{d:02}")
}
