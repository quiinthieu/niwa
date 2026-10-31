#[path = "../../../tests/support/mod.rs"]
mod support;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use niwa_garden::runtime::GardenSession;
use niwa_garden::tui::app::{OverlayKind, TuiApp};
use niwa_garden::tui::scheduler::{FrameScheduler, IDLE_TIMEOUT};
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use std::time::Duration;
use support::TestEnv;
use tg_core::clock::{FakeClock, Timestamp, TICKS_PER_DAY};
use tg_core::model::Stage;
use tg_core::species::Rarity;
use tg_render::ColorTier;
use tg_store::save::GardenStore;
use tg_store::snapshot::read_snapshot;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
    let buf = terminal.backend().buffer();
    let area = buf.area;
    let mut out = String::new();
    for y in 0..area.height {
        for x in 0..area.width {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

#[test]
fn test_backend_plants_fast_forwards_and_harvests_plus_move_compost_almanac_help_quit() {
    let env = TestEnv::new();
    // 2026-04-15T09:00:00Z -> Northern Spring (preferred season for White Clover).
    let clock = FakeClock::from_unix_secs(1_776_243_600);
    let session = GardenSession::open(env.paths().clone(), &clock, Some(0x4E49_5741_0008))
        .expect("open starter garden session");

    let mut app = TuiApp::new(session, ColorTier::NoColor, &clock).expect("init TuiApp");
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("init TestBackend");

    // Brand-new garden has no offline away summary to dismiss.
    assert_eq!(app.overlay_kind(), None);
    let initial_seeds: u32 = app.session().save().inventory.values().copied().sum();
    assert!(
        initial_seeds >= 4,
        "expected starter seed pouch to hold common seeds"
    );
    assert!(app.session().save().inventory.contains_key("clover"));

    app.render(&mut terminal).expect("render initial grid");
    let initial_screen = buffer_text(&terminal);
    assert!(initial_screen.contains("Niwa"));

    let mut mono = Duration::from_millis(0);

    // 1. Open Seed Pouch picker on Plot 0 (`'p'`) and plant `clover`.
    app.handle_key(key(KeyCode::Char('p')), mono, &clock)
        .expect("open seed picker");
    assert_eq!(app.overlay_kind(), Some(OverlayKind::SeedPicker));

    // Navigate seed picker until `clover` is highlighted, or select directly:
    // Inventory is a BTreeMap (`chamomile`, `clover`, `daisy`, `moss`), so index 1 is `clover`!
    app.handle_key(key(KeyCode::Down), mono, &clock)
        .expect("move down to clover");
    app.handle_key(key(KeyCode::Enter), mono, &clock)
        .expect("confirm planting clover");
    assert_eq!(app.overlay_kind(), None);

    let planted = app.session().garden().plots()[0]
        .plant()
        .expect("plot 0 must now contain clover");
    assert_eq!(planted.species_id, "clover");
    assert_eq!(planted.stage, Stage::Seed);
    assert_eq!(planted.total_gp, 0);

    // Pressing keys without time advancing must NEVER make a plant grow (PLAN §4.2).
    app.handle_key(key(KeyCode::Right), mono, &clock).unwrap();
    app.handle_key(key(KeyCode::Left), mono, &clock).unwrap();
    assert_eq!(
        app.session().garden().plots()[0].plant().unwrap().total_gp,
        0
    );

    // 2. Verify open TUI counts as an active signal (grants `base + sun` GP, not just `base`).
    clock.advance_ticks(1);
    let ticks_applied = app.sync_ticks(&clock).expect("sync 1 active TUI tick");
    assert_eq!(ticks_applied, 1);
    let gp_after_one_active_tick = app.session().garden().plots()[0].plant().unwrap().total_gp;
    let passive_only_gp = app.session().balance().compute_tick_gp(
        false, 0, 10_500, // clover 1.05x
        12_500, // spring preferred 1.25x
    );
    assert!(
        gp_after_one_active_tick > passive_only_gp,
        "open TUI tick must include active sun bonus ({gp_after_one_active_tick} vs passive {passive_only_gp})"
    );

    // 3. Fast-forward FakeClock until `clover` reaches `Stage::Blooming`.
    clock.advance_ticks(2_400);
    app.sync_ticks(&clock).expect("fast-forward to bloom");
    let bloomed = app.session().garden().plots()[0]
        .plant()
        .expect("clover still in plot 0");
    assert_eq!(bloomed.stage, Stage::Blooming);
    assert!(bloomed.bloomed_at.is_some());

    app.render(&mut terminal).expect("render blooming garden");
    let blooming_screen = buffer_text(&terminal);
    assert!(
        blooming_screen.contains("Blooming") || blooming_screen.contains("White Clover"),
        "expected blooming clover on screen, got:\n{blooming_screen}"
    );

    // 4. Harvest blooming `clover` on Plot 0 (`'h'`).
    let seeds_before_harvest: u32 = app.session().save().inventory.values().copied().sum();
    mono += Duration::from_millis(300);
    app.handle_key(key(KeyCode::Char('h')), mono, &clock)
        .expect("harvest blooming clover");

    assert!(
        app.session().garden().plots()[0].is_empty(),
        "harvesting must clear plot 0"
    );
    let seeds_after_harvest: u32 = app.session().save().inventory.values().copied().sum();
    assert!(
        seeds_after_harvest > seeds_before_harvest,
        "harvesting must drop at least 1 seed into inventory ({seeds_before_harvest} -> {seeds_after_harvest})"
    );
    let clover_almanac = app
        .session()
        .save()
        .almanac
        .get("clover")
        .expect("clover recorded in almanac");
    assert!(clover_almanac.first_bloomed_at.is_some());
    assert_eq!(clover_almanac.harvest_count, 1);

    // 5. Plant another seed in Plot 0, move it to Plot 1 (`'m'`), then compost it (`'c'` -> `'y'`).
    mono += Duration::from_millis(300);
    app.handle_key(key(KeyCode::Char('p')), mono, &clock)
        .unwrap();
    app.handle_key(key(KeyCode::Enter), mono, &clock).unwrap();
    let second_species_id = app.session().garden().plots()[0]
        .plant()
        .expect("second seed planted in plot 0")
        .species_id
        .clone();
    let second_rarity = app
        .session()
        .catalog()
        .get(&second_species_id)
        .map_or(Rarity::Common, |sp| sp.rarity);

    // Move Plot 0 -> Plot 1.
    app.handle_key(key(KeyCode::Char('m')), mono, &clock)
        .unwrap();
    assert!(app.is_moving());
    app.handle_key(key(KeyCode::Right), mono, &clock).unwrap();
    assert_eq!(app.selected_plot(), 1);
    app.handle_key(key(KeyCode::Enter), mono, &clock).unwrap();
    assert!(!app.is_moving());
    assert!(app.session().garden().plots()[0].is_empty());
    assert_eq!(
        app.session().garden().plots()[1]
            .plant()
            .unwrap()
            .species_id,
        second_species_id
    );

    // Compost Plot 1 -> returns 1 seed of the exact same rarity.
    let same_rarity_before: u32 = app
        .session()
        .save()
        .inventory
        .iter()
        .filter(|(id, _)| {
            app.session()
                .catalog()
                .get(id.as_str())
                .is_some_and(|sp| sp.rarity == second_rarity)
        })
        .map(|(_, &cnt)| cnt)
        .sum();

    app.handle_key(key(KeyCode::Char('c')), mono, &clock)
        .unwrap();
    assert_eq!(app.overlay_kind(), Some(OverlayKind::ConfirmCompost));
    app.handle_key(key(KeyCode::Char('y')), mono, &clock)
        .unwrap();
    assert_eq!(app.overlay_kind(), None);
    assert!(app.session().garden().plots()[1].is_empty());

    let same_rarity_after: u32 = app
        .session()
        .save()
        .inventory
        .iter()
        .filter(|(id, _)| {
            app.session()
                .catalog()
                .get(id.as_str())
                .is_some_and(|sp| sp.rarity == second_rarity)
        })
        .map(|(_, &cnt)| cnt)
        .sum();
    assert_eq!(
        same_rarity_after,
        same_rarity_before + 1,
        "composting must return exactly 1 seed of the same rarity"
    );

    // 6. Open Almanac (`'a'`) — shows discovered species + rarity words, NEVER numeric drop odds (`%`).
    app.handle_key(key(KeyCode::Char('a')), mono, &clock)
        .unwrap();
    assert_eq!(app.overlay_kind(), Some(OverlayKind::Almanac));
    app.render(&mut terminal).expect("render almanac");
    let almanac_screen = buffer_text(&terminal);
    assert!(almanac_screen.contains("Almanac"));
    assert!(almanac_screen.contains("White Clover"));
    assert!(almanac_screen.contains("???"));
    assert!(
        !almanac_screen.contains('%'),
        "Almanac UI must never display drop odds or percentages (PLAN §4.2), got:\n{almanac_screen}"
    );
    app.handle_key(key(KeyCode::Esc), mono, &clock).unwrap();
    assert_eq!(app.overlay_kind(), None);

    // 7. Open Help (`'?'`) and dismiss.
    app.handle_key(key(KeyCode::Char('?')), mono, &clock)
        .unwrap();
    assert_eq!(app.overlay_kind(), Some(OverlayKind::Help));
    app.render(&mut terminal).expect("render help");
    let help_screen = buffer_text(&terminal);
    assert!(help_screen.contains("Help"));
    app.handle_key(key(KeyCode::Esc), mono, &clock).unwrap();
    assert_eq!(app.overlay_kind(), None);

    // 8. Quit (`'q'`) and verify persisted `garden.json` and `snapshot.json`.
    assert!(!app.should_quit());
    app.handle_key(key(KeyCode::Char('q')), mono, &clock)
        .unwrap();
    assert!(app.should_quit());

    let loaded = GardenStore::new(env.paths().clone())
        .load()
        .expect("load persisted save")
        .expect("save exists");
    assert_eq!(loaded.save.almanac["clover"].harvest_count, 1);

    let snap = read_snapshot(env.paths())
        .expect("read snapshot")
        .expect("snapshot exists");
    assert_eq!(snap.planted_count, 0);
}

#[test]
fn opening_tui_after_offline_period_shows_away_summary_and_wakes_dormancy() {
    let env = TestEnv::new();
    let t0 = Timestamp::from_unix_secs(1_776_243_600);
    let clock = FakeClock::new(t0);

    {
        let mut session = GardenSession::open(env.paths().clone(), &clock, Some(1234)).unwrap();
        session.plant(0, "clover", t0).expect("plant clover");
    }

    // Advance offline for 8 days (past the 7-day dormancy window).
    clock.advance_ticks(8 * TICKS_PER_DAY);

    let session = GardenSession::open(env.paths().clone(), &clock, None).unwrap();
    let mut app = TuiApp::new(session, ColorTier::NoColor, &clock).unwrap();
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();

    // TUI opens with the "While you were away" overlay and wakes dormancy so new active ticks grow.
    assert_eq!(app.overlay_kind(), Some(OverlayKind::AwaySummary));
    assert!(
        !app.session().garden().is_dormant(),
        "opening the TUI must wake the dormant garden upon return"
    );

    app.render(&mut terminal).unwrap();
    let screen = buffer_text(&terminal);
    assert!(
        screen.contains("While you were away"),
        "expected away summary modal, got:\n{screen}"
    );

    app.handle_key(key(KeyCode::Enter), Duration::ZERO, &clock)
        .unwrap();
    assert_eq!(app.overlay_kind(), None);
}

#[test]
fn fps_cap_and_idle_zero_fps_drop_hold_under_spam_idle_timeout_and_focus_loss() {
    let env = TestEnv::new();
    let clock = FakeClock::from_unix_secs(1_776_243_600);
    let session = GardenSession::open(env.paths().clone(), &clock, Some(77)).unwrap();
    let mut app = TuiApp::new(session, ColorTier::NoColor, &clock).unwrap();
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();

    // 1. Over a 10-second active window (`0..10_000 ms`) with input spam every 50 ms (200 events),
    // default 4 fps (`>= 250 ms` frame interval) must draw at most 40 frames (never > 4 fps).
    let mut active_draws = 0_usize;
    for step_idx in 0..200_u64 {
        let mono = Duration::from_millis(step_idx * 50);
        let ev = Event::Key(key(KeyCode::Tab));
        if app
            .step(Some(ev), mono, &clock, &mut terminal)
            .expect("step active spam")
        {
            active_draws += 1;
        }
    }
    assert_eq!(
        active_draws, 40,
        "4 fps cap over 10.0s (0..9950ms) must allow exactly 40 draws (every 250ms), got {active_draws}"
    );

    // 2. Last user input occurred at `9_950 ms`. Once `30 s` (`IDLE_TIMEOUT`) elapses with no input
    // (`mono >= 39_950 ms`), effective fps MUST drop to 0 fps — zero draws across 5 full minutes
    // (`40_000 ms .. 340_000 ms`), even as 60s simulation ticks continue advancing in the background!
    let mut idle_draws = 0_usize;
    let tick_before_idle = app.session().garden().tick_index();
    let mut mono_ms = 40_000_u64;
    while mono_ms < 340_000 {
        if mono_ms.is_multiple_of(60_000) {
            clock.advance_ticks(1);
        }
        if app
            .step(None, Duration::from_millis(mono_ms), &clock, &mut terminal)
            .expect("step while idle")
        {
            idle_draws += 1;
        }
        mono_ms += 250;
    }
    assert_eq!(
        idle_draws, 0,
        "TUI must drop to 0 fps after 30s without user input, got {idle_draws} draws"
    );
    assert!(
        app.session().garden().tick_index() > tick_before_idle,
        "60s open-TUI simulation ticks must still advance while rendering sits at 0 fps"
    );

    // 3. New user input at `340_000 ms` wakes rendering immediately.
    let woke = app
        .step(
            Some(Event::Key(key(KeyCode::Tab))),
            Duration::from_millis(340_000),
            &clock,
            &mut terminal,
        )
        .unwrap();
    assert!(woke, "user input after idle must resume drawing");

    // 4. `FocusLost` immediately drops rendering to 0 fps without waiting 30 s,
    // and `FocusGained` wakes rendering again.
    app.step(
        Some(Event::FocusLost),
        Duration::from_millis(340_500),
        &clock,
        &mut terminal,
    )
    .unwrap();
    for offset in [341_000_u64, 341_500, 342_000, 345_000] {
        let drew = app
            .step(None, Duration::from_millis(offset), &clock, &mut terminal)
            .unwrap();
        assert!(!drew, "unfocused TUI must remain at 0 fps at {offset}ms");
    }

    let refocused = app
        .step(
            Some(Event::FocusGained),
            Duration::from_millis(346_000),
            &clock,
            &mut terminal,
        )
        .unwrap();
    assert!(refocused, "FocusGained must resume drawing");

    // 5. Direct `FrameScheduler` checks for 2 fps config and `next_poll_timeout` power budget.
    let mut sched_2fps = FrameScheduler::new(2);
    assert_eq!(sched_2fps.fps(), 2);
    assert_eq!(sched_2fps.frame_interval(), Duration::from_millis(500));
    assert_eq!(IDLE_TIMEOUT, Duration::from_secs(30));

    let mut draws_2fps = 0_usize;
    for ms in (0..10_000_u64).step_by(50) {
        let now = Duration::from_millis(ms);
        sched_2fps.on_user_input(now);
        if sched_2fps.should_draw(now) {
            sched_2fps.mark_drawn(now);
            draws_2fps += 1;
        }
    }
    assert_eq!(draws_2fps, 20, "2 fps cap over 10s must draw 20 frames");

    // When idle at 0 fps, `next_poll_timeout` must sleep all the way until the next 60s tick
    // rather than waking every 250ms/500ms!
    let idle_time = Duration::from_secs(60);
    assert!(sched_2fps.is_idle(idle_time));
    assert_eq!(sched_2fps.effective_fps(idle_time), 0);
    assert_eq!(
        sched_2fps.next_poll_timeout(idle_time, Duration::from_secs(47)),
        Duration::from_secs(47)
    );
}

#[test]
fn tui_renders_cleanly_across_tiny_compact_and_wide_viewports_and_all_color_tiers() {
    let env = TestEnv::new();
    let clock = FakeClock::from_unix_secs(1_776_243_600);

    for tier in ColorTier::ALL {
        let session = GardenSession::open(env.paths().clone(), &clock, Some(555)).unwrap();
        let mut app = TuiApp::new(session, tier, &clock).unwrap();

        for (w, h) in [
            (1_u16, 1_u16),
            (12, 5),
            (32, 10),
            (40, 12),
            (80, 24),
            (120, 40),
        ] {
            let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
            app.render(&mut terminal).expect("render main view");

            app.handle_key(key(KeyCode::Char('a')), Duration::ZERO, &clock)
                .unwrap();
            app.render(&mut terminal).expect("render almanac view");
            app.handle_key(key(KeyCode::Esc), Duration::ZERO, &clock)
                .unwrap();

            app.handle_key(key(KeyCode::Char('?')), Duration::ZERO, &clock)
                .unwrap();
            app.render(&mut terminal).expect("render help view");
            app.handle_key(key(KeyCode::Esc), Duration::ZERO, &clock)
                .unwrap();
        }
    }
}
