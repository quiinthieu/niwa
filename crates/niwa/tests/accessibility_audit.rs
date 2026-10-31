#[path = "../../../tests/support/mod.rs"]
mod support;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use niwa_garden::runtime::{format_status_human, GardenSession};
use niwa_garden::tui::app::{OverlayKind, TuiApp};
use niwa_garden::tui::settings::SettingsField;
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use std::time::Duration;
use support::TestEnv;
use tg_core::clock::{FakeClock, Timestamp, TICKS_PER_DAY};
use tg_core::model::{Plant, Plot, Stage};
use tg_core::sim::Garden;
use tg_core::species::{Rarity, SpeciesCatalog, SpriteSet};
use tg_render::layout::{render_garden, RenderOptions, Viewport};
use tg_render::sprites::SpriteBank;
use tg_render::width::strip_ansi;
use tg_render::ColorTier;
use tg_store::save::{GardenSave, GardenStore};
use tg_store::snapshot::read_snapshot;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
    let buf = terminal.backend().buffer();
    let mut lines = Vec::with_capacity(usize::from(buf.area.height));
    for y in 0..buf.area.height {
        let mut line = String::with_capacity(usize::from(buf.area.width));
        for x in 0..buf.area.width {
            line.push_str(buf[(x, y)].symbol());
        }
        lines.push(line.trim_end().to_string());
    }
    lines.join("\n")
}

fn assert_strictly_7bit_ascii(label: &str, text: &str) {
    for (line_idx, line) in text.lines().enumerate() {
        for ch in line.chars() {
            let code = ch as u32;
            assert!(
                (0x20..=0x7E).contains(&code),
                "{label} leaked non-7-bit-ASCII character {ch:?} (U+{code:04X}) on line {}: {:?}",
                line_idx + 1,
                line
            );
        }
    }
}

#[test]
fn ascii_mode_renders_strictly_7bit_printable_ascii_across_every_tui_screen_modal_and_cli_view() {
    let env = TestEnv::new();
    // 2026-01-15 UTC (Winter in northern hemisphere) so `sakura` is an out-of-season specialist.
    let t0 = Timestamp::from_unix_secs(1_768_435_200);
    let clock = FakeClock::new(t0);

    {
        let mut session = GardenSession::open(env.paths().clone(), &clock, Some(4242)).unwrap();
        session
            .set_config_key("sprites", "ascii", &clock)
            .expect("enable 7-bit ascii sprite mode");
        session.plant(0, "clover", t0).expect("plant clover");
        session.plant(1, "daisy", t0).expect("plant daisy");
    }

    // Advance 8 days offline so `AwaySummary` modal is queued on TUI open.
    clock.advance_ticks(8 * TICKS_PER_DAY);

    let session = GardenSession::open(env.paths().clone(), &clock, None).unwrap();
    let mut app = TuiApp::new(session, ColorTier::NoColor, &clock).unwrap();
    let mut mono = Duration::ZERO;

    // 1. "While you were away" modal in ASCII mode.
    assert_eq!(app.overlay_kind(), Some(OverlayKind::AwaySummary));
    {
        let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
        app.render(&mut term).unwrap();
        let screen = buffer_text(&term);
        assert!(screen.contains("While you were away"));
        assert_strictly_7bit_ascii("AwaySummary modal", &screen);
    }
    app.handle_key(key(KeyCode::Enter), mono, &clock).unwrap();
    assert_eq!(app.overlay_kind(), None);

    // 2. Main garden view across compact list (32x10), compact grid (40x12),
    // medium grid (60x18), standard grid (80x24), and spacious grid (120x40).
    for (w, h) in [(32, 10), (40, 12), (60, 18), (80, 24), (120, 40)] {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        app.render(&mut term).unwrap();
        let screen = buffer_text(&term);
        assert_strictly_7bit_ascii(&format!("Main TUI {w}x{h}"), &screen);
    }

    // 3. Status messages for occupied/empty plot actions + Move mode.
    let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();

    // Occupied plot 0: pressing 'p' warns plot already holds a plant.
    mono += Duration::from_millis(300);
    app.handle_key(key(KeyCode::Char('p')), mono, &clock)
        .unwrap();
    app.render(&mut term).unwrap();
    assert_strictly_7bit_ascii("Occupied plot plant warning", &buffer_text(&term));

    // Move mode from plot 0 -> plot 2 (empty).
    mono += Duration::from_millis(300);
    app.handle_key(key(KeyCode::Char('m')), mono, &clock)
        .unwrap();
    assert!(app.is_moving());
    for (w, h) in [(32, 10), (40, 12), (80, 24)] {
        let mut mv_term = Terminal::new(TestBackend::new(w, h)).unwrap();
        app.render(&mut mv_term).unwrap();
        assert_strictly_7bit_ascii(&format!("Move mode {w}x{h}"), &buffer_text(&mv_term));
    }
    app.handle_key(key(KeyCode::Esc), mono, &clock).unwrap();

    // Navigate to empty plot 2: test empty compost/move warnings, then open SeedPicker.
    app.handle_key(key(KeyCode::Char('3')), mono, &clock)
        .unwrap();
    app.handle_key(key(KeyCode::Char('c')), mono, &clock)
        .unwrap();
    app.render(&mut term).unwrap();
    assert_strictly_7bit_ascii("Empty plot compost warning", &buffer_text(&term));

    app.handle_key(key(KeyCode::Char('m')), mono, &clock)
        .unwrap();
    app.render(&mut term).unwrap();
    assert_strictly_7bit_ascii("Empty plot move warning", &buffer_text(&term));

    // 4. SeedPicker modal.
    app.handle_key(key(KeyCode::Char('p')), mono, &clock)
        .unwrap();
    assert_eq!(app.overlay_kind(), Some(OverlayKind::SeedPicker));
    app.render(&mut term).unwrap();
    assert_strictly_7bit_ascii("SeedPicker modal", &buffer_text(&term));
    // Plant chamomile (non-blooming) and try harvesting it to test PlotNotBlooming message.
    app.handle_key(key(KeyCode::Enter), mono, &clock).unwrap();
    assert_eq!(app.overlay_kind(), None);
    app.handle_key(key(KeyCode::Char('h')), mono, &clock)
        .unwrap();
    app.render(&mut term).unwrap();
    assert_strictly_7bit_ascii("PlotNotBlooming warning", &buffer_text(&term));

    // 5. ConfirmCompost modal.
    app.handle_key(key(KeyCode::Char('c')), mono, &clock)
        .unwrap();
    assert_eq!(app.overlay_kind(), Some(OverlayKind::ConfirmCompost));
    app.render(&mut term).unwrap();
    assert_strictly_7bit_ascii("ConfirmCompost modal", &buffer_text(&term));
    app.handle_key(key(KeyCode::Esc), mono, &clock).unwrap();

    // 6. Almanac modal across all 16 species rows (discovered & undiscovered).
    app.handle_key(key(KeyCode::Char('a')), mono, &clock)
        .unwrap();
    assert_eq!(app.overlay_kind(), Some(OverlayKind::Almanac));
    for idx in 0..app.session().catalog().len() {
        app.render(&mut term).unwrap();
        assert_strictly_7bit_ascii(&format!("Almanac modal row {idx}"), &buffer_text(&term));
        app.handle_key(key(KeyCode::Down), mono, &clock).unwrap();
    }
    app.handle_key(key(KeyCode::Esc), mono, &clock).unwrap();

    // 7. Settings modal across all 17 rows + inline edit + validation error + SignalConsent modal.
    let mut wide_term = Terminal::new(TestBackend::new(112, 28)).unwrap();
    app.handle_key(key(KeyCode::Char('s')), mono, &clock)
        .unwrap();
    assert_eq!(app.overlay_kind(), Some(OverlayKind::Settings));
    for idx in 0..SettingsField::ALL.len() {
        app.render(&mut wide_term).unwrap();
        assert_strictly_7bit_ascii(
            &format!(
                "Settings modal row {idx} ({})",
                SettingsField::ALL[idx].key()
            ),
            &buffer_text(&wide_term),
        );

        let current_field = app.selected_settings_field().unwrap();
        if current_field == SettingsField::DormantAfterDays {
            // Enter inline edit mode, type invalid "0", submit to render validation error banner.
            app.handle_key(key(KeyCode::Enter), mono, &clock).unwrap();
            app.render(&mut wide_term).unwrap();
            assert_strictly_7bit_ascii("Settings inline edit mode", &buffer_text(&wide_term));

            app.handle_key(
                KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
                mono,
                &clock,
            )
            .unwrap();
            app.handle_key(key(KeyCode::Char('0')), mono, &clock)
                .unwrap();
            app.handle_key(key(KeyCode::Enter), mono, &clock).unwrap();
            app.render(&mut wide_term).unwrap();
            assert_strictly_7bit_ascii("Settings validation error", &buffer_text(&wide_term));
            app.handle_key(key(KeyCode::Esc), mono, &clock).unwrap();
        } else if current_field == SettingsField::SignalActivityEnabled {
            app.handle_key(key(KeyCode::Right), mono, &clock).unwrap();
            assert_eq!(app.overlay_kind(), Some(OverlayKind::SignalConsent));
            app.render(&mut wide_term).unwrap();
            assert_strictly_7bit_ascii("SignalConsent modal", &buffer_text(&wide_term));
            app.handle_key(key(KeyCode::Esc), mono, &clock).unwrap();
            assert_eq!(app.overlay_kind(), Some(OverlayKind::Settings));
        }

        app.handle_key(key(KeyCode::Down), mono, &clock).unwrap();
    }
    app.handle_key(key(KeyCode::Esc), mono, &clock).unwrap();

    // 8. Help modal.
    app.handle_key(key(KeyCode::Char('?')), mono, &clock)
        .unwrap();
    assert_eq!(app.overlay_kind(), Some(OverlayKind::Help));
    app.render(&mut term).unwrap();
    assert_strictly_7bit_ascii("Help modal", &buffer_text(&term));
    app.handle_key(key(KeyCode::Esc), mono, &clock).unwrap();

    // 9. CLI human status, status report summary, peek frame, and persisted snapshot.json.
    let report = app.session().status_report();
    assert_strictly_7bit_ascii("StatusReport.glyph", &report.glyph);
    assert_strictly_7bit_ascii("StatusReport.summary", &report.summary);
    let human_status = format_status_human(&report, app.session().catalog());
    assert_strictly_7bit_ascii("niwa status human output", &human_status);

    let peek = app
        .session()
        .render_frame(
            Viewport::new(80, 24),
            Some(SpriteSet::Ascii),
            ColorTier::NoColor,
        )
        .to_plain_string();
    assert_strictly_7bit_ascii("niwa peek --ascii", &peek);

    let snap = read_snapshot(env.paths()).unwrap().unwrap();
    assert_strictly_7bit_ascii("snapshot.json glyph", &snap.glyph);
    assert_strictly_7bit_ascii("snapshot.json summary", &snap.summary);
    assert_strictly_7bit_ascii("snapshot.json compact_frame", &snap.compact_frame);
}

#[test]
fn no_color_only_state_cues_across_all_garden_and_tui_states() {
    let catalog = SpeciesCatalog::load_default();
    let sprites = SpriteBank::load_default();
    // 2026-01-15 UTC (Winter in northern hemisphere).
    let t0 = Timestamp::from_unix_secs(1_768_435_200);

    // Build a deterministic garden containing all 5 stages, all 4 rarities, an out-of-season
    // specialist seed (`sakura` in Winter), and empty plots.
    let make_garden = |dormant: bool| -> Garden {
        let specs: [Option<(&str, Stage, u64, u64)>; 8] = [
            Some(("sakura", Stage::Seed, 0, 0)),
            Some(("clover", Stage::Sprout, 40_000, 76_000)),
            Some(("lavender", Stage::Young, 120_000, 276_000)),
            Some(("red_maple", Stage::Mature, 240_000, 740_000)),
            Some(("frostbell", Stage::Blooming, 0, 1_402_000)),
            None,
            Some(("daisy", Stage::Seed, 15_000, 15_000)),
            None,
        ];
        let plots: Vec<Plot> = specs
            .into_iter()
            .map(|spec| match spec {
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
            99,
            plots,
            t0,
            t0,
            0,
            if dormant { 10_080 } else { 0 },
            dormant,
        )
    };

    let active_garden = make_garden(false);
    let dormant_garden = make_garden(true);

    // 1. Pure renderer: stripping ANSI from Truecolor must preserve 100% of textual information
    // in NoColor, and Active vs Dormant + all 5 stages + empty plots must be distinguishable in
    // plain text at EVERY viewport size (32x10, 40x12, 60x18 medium, 80x24 standard, 120x40 spacious).
    for set in SpriteSet::ALL {
        for (w, h) in [(32, 10), (40, 12), (60, 18), (80, 24), (120, 40)] {
            let vp = Viewport::new(w, h);
            let active_plain = render_garden(
                &active_garden,
                &catalog,
                &sprites,
                vp,
                RenderOptions {
                    sprites: set,
                    color: ColorTier::NoColor,
                    season: tg_core::season::Season::Winter,
                },
            )
            .to_plain_string();
            let active_truecolor = render_garden(
                &active_garden,
                &catalog,
                &sprites,
                vp,
                RenderOptions {
                    sprites: set,
                    color: ColorTier::Truecolor,
                    season: tg_core::season::Season::Winter,
                },
            )
            .to_ansi_string();
            assert_eq!(
                strip_ansi(&active_truecolor),
                active_plain,
                "Truecolor must not add or hide textual symbols relative to NoColor ({set:?} {w}x{h})"
            );

            let dormant_plain = render_garden(
                &dormant_garden,
                &catalog,
                &sprites,
                vp,
                RenderOptions {
                    sprites: set,
                    color: ColorTier::NoColor,
                    season: tg_core::season::Season::Winter,
                },
            )
            .to_plain_string();

            assert_ne!(
                active_plain, dormant_plain,
                "Active vs Dormant must differ in plain monochrome text at {w}x{h} ({set:?})"
            );
            assert!(
                dormant_plain.contains("[Dormant]"),
                "Dormant frame must include explicit '[Dormant]' tag at {w}x{h} ({set:?})"
            );
            // Medium (60x18), Standard (80x24), and Spacious (120x40) cells also show per-card rest text.
            if h >= 18 {
                assert!(
                    dormant_plain.contains("[Rest]") || dormant_plain.contains("Resting"),
                    "Dormant multi-line plot cards must show explicit '[Rest]' / 'Resting' text at {w}x{h} ({set:?}), got:\n{dormant_plain}"
                );
            }
        }
    }

    // 2. TUI non-color state cues across EVERY viewport (32x10, 40x12, 80x24, 120x40):
    //    - Selected plot cursor `>#1` and Move-source marker `*#1` appear in plain text even on
    //      compact 32x10 and 40x12 layouts (not bold/underline only!).
    //    - Out-of-season specialist seed (`sakura` in Winter) shows an explicit non-color season
    //      wait cue (`awaits spring`) in the inspector bar, whereas year-round `daisy` does not.
    //    - All 4 Rarity labels (`common`, `uncommon`, `rare`, `mythic`) appear as explicit words.
    let env = TestEnv::new();
    let clock = FakeClock::new(t0);
    {
        let mut save = GardenSave::from_garden(&active_garden);
        save.inventory.insert("sakura".to_string(), 1);
        save.inventory.insert("daisy".to_string(), 1);
        GardenStore::new(env.paths().clone()).save(&save).unwrap();
    }

    let session = GardenSession::open(env.paths().clone(), &clock, None).unwrap();
    let mut app = TuiApp::new(session, ColorTier::NoColor, &clock).unwrap();

    // Check selected plot `>#1` cursor on ALL viewports including 32x10 and 40x12!
    for (w, h) in [(32, 12), (40, 14), (80, 24), (120, 40)] {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        app.render(&mut term).unwrap();
        let screen = buffer_text(&term);
        assert!(
            screen.contains(">#1"),
            "Selected plot 1 must carry explicit plain-text cursor '>#1' at {w}x{h}, got:\n{screen}"
        );
    }

    // Check out-of-season specialist seed cue on Plot 1 (`sakura` in Winter) vs Plot 7 (`daisy`).
    {
        let mut term = Terminal::new(TestBackend::new(90, 24)).unwrap();
        app.render(&mut term).unwrap();
        let plot1_screen = buffer_text(&term);
        assert!(
            plot1_screen.contains("awaits spring"),
            "Out-of-season specialist seed must expose explicit plain-text '(awaits spring)' cue in inspector, got:\n{plot1_screen}"
        );
        assert!(plot1_screen.contains("[mythic]"));

        // Enter Move mode on Plot 1 (`*#1`) and move cursor to Plot 2 (`>#2`):
        // both `*#1` and `>#2` must appear in plain text across 32x12, 40x14, and 80x24!
        app.handle_key(key(KeyCode::Char('m')), Duration::from_millis(300), &clock)
            .unwrap();
        app.handle_key(key(KeyCode::Right), Duration::from_millis(600), &clock)
            .unwrap();
        for (w, h) in [(32, 12), (40, 14), (80, 24)] {
            let mut mv_term = Terminal::new(TestBackend::new(w, h)).unwrap();
            app.render(&mut mv_term).unwrap();
            let mv_screen = buffer_text(&mv_term);
            assert!(
                mv_screen.contains("*#1") && mv_screen.contains(">#2"),
                "Move source '*#1' and selected destination '>#2' must both be explicit in plain text at {w}x{h}, got:\n{mv_screen}"
            );
        }
        app.handle_key(key(KeyCode::Esc), Duration::from_millis(900), &clock)
            .unwrap();

        // Check rarities across plots 2 (common), 3 (uncommon), 4 (rare), and plot 7 (year-round seed).
        let expected_rarities = [
            ('2', Rarity::Common.label()),
            ('3', Rarity::Uncommon.label()),
            ('4', Rarity::Rare.label()),
        ];
        for (digit, rarity_word) in expected_rarities {
            app.handle_key(key(KeyCode::Char(digit)), Duration::from_secs(1), &clock)
                .unwrap();
            app.render(&mut term).unwrap();
            let s = buffer_text(&term);
            assert!(
                s.contains(&format!("[{rarity_word}]")),
                "Inspector for plot {digit} must include explicit rarity word '[{rarity_word}]', got:\n{s}"
            );
        }

        app.handle_key(key(KeyCode::Char('7')), Duration::from_secs(2), &clock)
            .unwrap();
        app.render(&mut term).unwrap();
        let plot7_screen = buffer_text(&term);
        assert!(
            !plot7_screen.contains("awaits"),
            "Year-round seed ('daisy') must not show an out-of-season wait tag, got:\n{plot7_screen}"
        );
    }
}
