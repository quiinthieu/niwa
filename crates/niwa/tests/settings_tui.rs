#[path = "../../../tests/support/mod.rs"]
mod support;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use niwa_garden::cli::{execute_with, Cli, Commands, ConfigCommand};
use niwa_garden::runtime::GardenSession;
use niwa_garden::tui::app::{OverlayKind, TuiApp};
use niwa_garden::tui::settings::SettingsField;
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use std::collections::BTreeSet;
use std::time::Duration;
use support::TestEnv;
use tg_core::clock::{FakeClock, Timestamp};
use tg_render::ColorTier;
use tg_signals::consent::ConsentCatalog;
use tg_signals::SignalKind;
use tg_store::config::{AppConfig, ConfigStore};
use tg_store::snapshot::read_snapshot;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(ch: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL)
}

fn type_str(app: &mut TuiApp, text: &str, mono: Duration, clock: &FakeClock) {
    for ch in text.chars() {
        app.handle_key(key(KeyCode::Char(ch)), mono, clock)
            .expect("type character into settings editor");
    }
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

fn select_field(app: &mut TuiApp, target: SettingsField, mono: Duration, clock: &FakeClock) {
    assert_eq!(app.overlay_kind(), Some(OverlayKind::Settings));
    // Jump to top row via Home, then step Down to target index.
    app.handle_key(key(KeyCode::Home), mono, clock)
        .expect("jump to first settings row");
    let target_index = SettingsField::ALL
        .iter()
        .position(|&field| field == target)
        .expect("target SettingsField in ALL");
    for _ in 0..target_index {
        app.handle_key(key(KeyCode::Down), mono, clock)
            .expect("step down to settings row");
    }
    assert_eq!(app.selected_settings_field(), Some(target));
}

#[test]
fn settings_field_list_covers_every_canonical_config_key() {
    let field_keys: BTreeSet<&'static str> = SettingsField::ALL
        .into_iter()
        .map(SettingsField::key)
        .collect();
    let canonical_keys: BTreeSet<&'static str> = AppConfig::ALL_KEYS.into_iter().collect();
    assert_eq!(
        field_keys, canonical_keys,
        "TUI SettingsField::ALL must cover every single AppConfig::ALL_KEYS entry 1-to-1"
    );
}

#[test]
fn tui_settings_sets_every_config_key_and_enforces_shared_consent_disclosure() {
    let env = TestEnv::new();
    let t0 = Timestamp::from_unix_secs(1_776_243_600);
    let clock = FakeClock::new(t0);

    let mut session =
        GardenSession::open(env.paths().clone(), &clock, Some(0x4E49_5741_0015)).unwrap();
    session
        .plant(0, "clover", t0)
        .expect("plant starter clover");

    let mut app = TuiApp::new(session, ColorTier::NoColor, &clock).expect("init TuiApp");
    let mut terminal = Terminal::new(TestBackend::new(110, 36)).expect("init TestBackend");
    let mono = Duration::from_millis(100);

    // Welcome status bar and Help screen advertise `[s]` Settings.
    assert!(
        app.status_message().contains("[s]ettings"),
        "welcome bar should mention [s]ettings, got: {}",
        app.status_message()
    );
    app.handle_key(key(KeyCode::Char('?')), mono, &clock)
        .unwrap();
    app.render(&mut terminal).unwrap();
    let help_screen = buffer_text(&terminal);
    assert!(
        help_screen.contains("Settings"),
        "Help modal must list 's' Settings shortcut:\n{help_screen}"
    );
    app.handle_key(key(KeyCode::Esc), mono, &clock).unwrap();

    // Open Settings (`'s'`).
    app.handle_key(key(KeyCode::Char('s')), mono, &clock)
        .expect("open settings overlay");
    assert_eq!(app.overlay_kind(), Some(OverlayKind::Settings));

    // 1. `sprites`: unicode -> ascii (updates snapshot glyph from Unicode to 7-bit ASCII).
    select_field(&mut app, SettingsField::Sprites, mono, &clock);
    app.handle_key(key(KeyCode::Enter), mono, &clock).unwrap();
    assert_eq!(
        ConfigStore::new(env.paths().clone())
            .load()
            .unwrap()
            .get_key("sprites")
            .unwrap(),
        "ascii"
    );
    let snap_ascii = read_snapshot(env.paths()).unwrap().unwrap();
    assert!(
        snap_ascii.glyph.is_ascii(),
        "changing sprites to ascii must refresh snapshot.json to ASCII glyph, got {}",
        snap_ascii.glyph
    );

    // 2. `color`: auto -> truecolor -> 256 -> 16.
    select_field(&mut app, SettingsField::Color, mono, &clock);
    app.handle_key(key(KeyCode::Right), mono, &clock).unwrap(); // truecolor
    app.handle_key(key(KeyCode::Right), mono, &clock).unwrap(); // 256
    app.handle_key(key(KeyCode::Right), mono, &clock).unwrap(); // 16
    assert_eq!(
        ConfigStore::new(env.paths().clone())
            .load()
            .unwrap()
            .get_key("color")
            .unwrap(),
        "16"
    );
    if std::env::var_os("NO_COLOR").is_none() {
        assert_eq!(app.color_tier(), ColorTier::Ansi16);
    }

    // 3. `fps`: 4 -> 2 (updates FrameScheduler live).
    select_field(&mut app, SettingsField::Fps, mono, &clock);
    app.handle_key(key(KeyCode::Left), mono, &clock).unwrap(); // 3
    app.handle_key(key(KeyCode::Left), mono, &clock).unwrap(); // 2
    assert_eq!(app.scheduler().fps(), 2);
    assert_eq!(app.scheduler().frame_interval(), Duration::from_millis(500));

    // 4. `season_mode`: calendar -> garden.
    select_field(&mut app, SettingsField::SeasonMode, mono, &clock);
    app.handle_key(key(KeyCode::Enter), mono, &clock).unwrap();

    // 5. `hemisphere`: north -> south.
    select_field(&mut app, SettingsField::Hemisphere, mono, &clock);
    app.handle_key(key(KeyCode::Enter), mono, &clock).unwrap();

    // 6. `dormant_after_days`: inline edit to 14 (and reject invalid 0 first).
    select_field(&mut app, SettingsField::DormantAfterDays, mono, &clock);
    app.handle_key(key(KeyCode::Enter), mono, &clock).unwrap();
    app.handle_key(ctrl('u'), mono, &clock).unwrap();
    type_str(&mut app, "0", mono, &clock);
    app.handle_key(key(KeyCode::Enter), mono, &clock).unwrap();
    assert_eq!(
        ConfigStore::new(env.paths().clone())
            .load()
            .unwrap()
            .dormant_after_days,
        7,
        "invalid dormant_after_days = 0 must not overwrite config.toml"
    );
    app.handle_key(ctrl('u'), mono, &clock).unwrap();
    type_str(&mut app, "14", mono, &clock);
    app.handle_key(key(KeyCode::Enter), mono, &clock).unwrap();

    // 7. `prompt.style`: glyph -> emoji.
    select_field(&mut app, SettingsField::PromptStyle, mono, &clock);
    app.handle_key(key(KeyCode::Right), mono, &clock).unwrap();

    // 8. `menubar.show_title`: true -> false.
    select_field(&mut app, SettingsField::MenubarShowTitle, mono, &clock);
    app.handle_key(key(KeyCode::Enter), mono, &clock).unwrap();

    // 9. `signals.paused`: false -> true.
    select_field(&mut app, SettingsField::SignalsPaused, mono, &clock);
    app.handle_key(key(KeyCode::Enter), mono, &clock).unwrap();

    // 10 & 11. `signals.activity.enabled` + `signals.activity.consented_at`:
    // Enabling MUST open SignalConsent showing the exact shared `data/consent.toml` text!
    let consent_catalog = ConsentCatalog::load_default();
    clock.set(Timestamp::from_unix_secs(1_776_250_001));
    select_field(&mut app, SettingsField::SignalActivityEnabled, mono, &clock);
    app.handle_key(key(KeyCode::Enter), mono, &clock).unwrap();
    assert_eq!(app.overlay_kind(), Some(OverlayKind::SignalConsent));

    app.render(&mut terminal).unwrap();
    let consent_screen = buffer_text(&terminal);
    let expected_activity_body = consent_catalog
        .signal(SignalKind::Activity)
        .render_disclosure_body(SignalKind::Activity);
    for expected_line in expected_activity_body.lines() {
        let trimmed = expected_line.trim();
        if !trimmed.is_empty() {
            assert!(
                consent_screen.contains(trimmed),
                "TUI consent modal missing shared consent.toml line '{trimmed}':\n{consent_screen}"
            );
        }
    }

    // Declining (`'n'`) must NOT enable or stamp consent!
    app.handle_key(key(KeyCode::Char('n')), mono, &clock)
        .unwrap();
    assert_eq!(app.overlay_kind(), Some(OverlayKind::Settings));
    let after_decline = ConfigStore::new(env.paths().clone()).load().unwrap();
    assert!(!after_decline.signals.activity.enabled);
    assert_eq!(after_decline.signals.activity.consented_at, None);

    // Accepting (`'y'`) enables and stamps `consented_at = clock.now()`.
    app.handle_key(key(KeyCode::Enter), mono, &clock).unwrap();
    assert_eq!(app.overlay_kind(), Some(OverlayKind::SignalConsent));
    app.handle_key(key(KeyCode::Char('y')), mono, &clock)
        .unwrap();
    assert_eq!(app.overlay_kind(), Some(OverlayKind::Settings));
    let after_accept = ConfigStore::new(env.paths().clone()).load().unwrap();
    assert!(after_accept.signals.activity.enabled);
    assert_eq!(
        after_accept.signals.activity.consented_at,
        Some(Timestamp::from_unix_secs(1_776_250_001))
    );

    // Also verify `signals.activity.consented_at` can be edited directly.
    select_field(
        &mut app,
        SettingsField::SignalActivityConsentedAt,
        mono,
        &clock,
    );
    app.handle_key(key(KeyCode::Enter), mono, &clock).unwrap();
    app.handle_key(ctrl('u'), mono, &clock).unwrap();
    type_str(&mut app, "1776250111", mono, &clock);
    app.handle_key(key(KeyCode::Enter), mono, &clock).unwrap();

    // 12 & 13. `signals.git.enabled` + `signals.git.consented_at`:
    clock.set(Timestamp::from_unix_secs(1_776_250_222));
    select_field(&mut app, SettingsField::SignalGitEnabled, mono, &clock);
    app.handle_key(key(KeyCode::Enter), mono, &clock).unwrap();
    assert_eq!(app.overlay_kind(), Some(OverlayKind::SignalConsent));
    app.render(&mut terminal).unwrap();
    let git_consent_screen = buffer_text(&terminal);
    assert!(git_consent_screen.contains(&consent_catalog.signal(SignalKind::Git).prompt));
    app.handle_key(key(KeyCode::Enter), mono, &clock).unwrap();
    assert_eq!(app.overlay_kind(), Some(OverlayKind::Settings));

    select_field(&mut app, SettingsField::SignalGitConsentedAt, mono, &clock);
    app.handle_key(key(KeyCode::Enter), mono, &clock).unwrap();
    app.handle_key(ctrl('u'), mono, &clock).unwrap();
    type_str(&mut app, "1776250333", mono, &clock);
    app.handle_key(key(KeyCode::Enter), mono, &clock).unwrap();

    // 14 & 15. `signals.shell.enabled` + `signals.shell.consented_at`:
    clock.set(Timestamp::from_unix_secs(1_776_250_444));
    select_field(&mut app, SettingsField::SignalShellEnabled, mono, &clock);
    app.handle_key(key(KeyCode::Enter), mono, &clock).unwrap();
    assert_eq!(app.overlay_kind(), Some(OverlayKind::SignalConsent));
    app.render(&mut terminal).unwrap();
    let shell_consent_screen = buffer_text(&terminal);
    assert!(shell_consent_screen.contains("Shell commands"));
    app.handle_key(key(KeyCode::Char('Y')), mono, &clock)
        .unwrap();
    assert_eq!(app.overlay_kind(), Some(OverlayKind::Settings));

    // Disable shell and clear `signals.shell.consented_at` via `'x'` -> `"none"`, then set custom ts.
    app.handle_key(key(KeyCode::Enter), mono, &clock).unwrap(); // true -> false
    select_field(
        &mut app,
        SettingsField::SignalShellConsentedAt,
        mono,
        &clock,
    );
    app.handle_key(key(KeyCode::Char('x')), mono, &clock)
        .unwrap();
    assert_eq!(
        ConfigStore::new(env.paths().clone())
            .load()
            .unwrap()
            .get_key("signals.shell.consented_at")
            .unwrap(),
        "none"
    );
    // Re-enable shell via consent and keep its timestamp.
    select_field(&mut app, SettingsField::SignalShellEnabled, mono, &clock);
    app.handle_key(key(KeyCode::Char(' ')), mono, &clock)
        .unwrap();
    app.handle_key(key(KeyCode::Char('y')), mono, &clock)
        .unwrap();

    // 16. `git.repos`: inline edit with comma-separated repository paths.
    select_field(&mut app, SettingsField::GitRepos, mono, &clock);
    app.handle_key(key(KeyCode::Enter), mono, &clock).unwrap();
    type_str(&mut app, "/tmp/niwa-alpha, /tmp/niwa-beta", mono, &clock);
    app.handle_key(key(KeyCode::Enter), mono, &clock).unwrap();

    // 17. `git.author_email`: reject malformed email without `@`, then save valid email.
    select_field(&mut app, SettingsField::GitAuthorEmail, mono, &clock);
    app.handle_key(key(KeyCode::Enter), mono, &clock).unwrap();
    type_str(&mut app, "invalid-email", mono, &clock);
    app.handle_key(key(KeyCode::Enter), mono, &clock).unwrap();
    assert!(
        app.status_message().contains('@') || app.status_message().contains("invalid"),
        "expected validation error for malformed email, got: {}",
        app.status_message()
    );
    app.handle_key(ctrl('u'), mono, &clock).unwrap();
    type_str(&mut app, "gardener@example.com", mono, &clock);
    app.handle_key(key(KeyCode::Enter), mono, &clock).unwrap();

    // Close Settings (`Esc`).
    app.handle_key(key(KeyCode::Esc), mono, &clock).unwrap();
    assert_eq!(app.overlay_kind(), None);

    // Reload `config.toml` from disk and verify ALL 17 canonical keys persisted!
    let persisted = ConfigStore::new(env.paths().clone()).load().unwrap();
    assert_eq!(persisted.get_key("sprites").unwrap(), "ascii");
    assert_eq!(persisted.get_key("color").unwrap(), "16");
    assert_eq!(persisted.get_key("fps").unwrap(), "2");
    assert_eq!(persisted.get_key("season_mode").unwrap(), "garden");
    assert_eq!(persisted.get_key("hemisphere").unwrap(), "south");
    assert_eq!(persisted.get_key("dormant_after_days").unwrap(), "14");
    assert_eq!(persisted.get_key("prompt.style").unwrap(), "emoji");
    assert_eq!(persisted.get_key("menubar.show_title").unwrap(), "false");
    assert_eq!(persisted.get_key("signals.paused").unwrap(), "true");
    assert_eq!(
        persisted.get_key("signals.activity.enabled").unwrap(),
        "true"
    );
    assert_eq!(
        persisted.get_key("signals.activity.consented_at").unwrap(),
        "1776250111"
    );
    assert_eq!(persisted.get_key("signals.git.enabled").unwrap(), "true");
    assert_eq!(
        persisted.get_key("signals.git.consented_at").unwrap(),
        "1776250333"
    );
    assert_eq!(persisted.get_key("signals.shell.enabled").unwrap(), "true");
    assert_eq!(
        persisted.get_key("signals.shell.consented_at").unwrap(),
        "1776250444"
    );
    assert_eq!(
        persisted.get_key("git.repos").unwrap(),
        "/tmp/niwa-alpha,/tmp/niwa-beta"
    );
    assert_eq!(
        persisted.get_key("git.author_email").unwrap(),
        "gardener@example.com"
    );
}

#[test]
fn cli_config_set_refreshes_existing_garden_snapshot_for_menubar_and_prompt() {
    let env = TestEnv::new();
    let t0 = Timestamp::from_unix_secs(1_776_243_600);
    let clock = FakeClock::new(t0);

    {
        let mut session = GardenSession::open(env.paths().clone(), &clock, Some(99)).unwrap();
        session.plant(0, "clover", t0).unwrap();
    }

    let before = read_snapshot(env.paths()).unwrap().unwrap();
    assert!(
        !before.glyph.is_ascii(),
        "default unicode glyph should be non-ASCII, got {}",
        before.glyph
    );

    execute_with(
        Cli {
            no_color: false,
            command: Some(Commands::Config {
                action: ConfigCommand::Set {
                    key: "sprites".to_string(),
                    value: "ascii".to_string(),
                },
            }),
        },
        env.paths().clone(),
        &clock,
    )
    .expect("niwa config set sprites ascii");

    let after = read_snapshot(env.paths()).unwrap().unwrap();
    assert!(
        after.glyph.is_ascii(),
        "niwa config set sprites ascii must refresh snapshot.json when garden.json exists, got {}",
        after.glyph
    );
}

#[test]
fn settings_and_consent_overlays_render_cleanly_across_tiny_and_wide_viewports() {
    let env = TestEnv::new();
    let clock = FakeClock::from_unix_secs(1_776_243_600);
    let session = GardenSession::open(env.paths().clone(), &clock, Some(321)).unwrap();
    let mut app = TuiApp::new(session, ColorTier::NoColor, &clock).unwrap();
    let mono = Duration::ZERO;

    for (w, h) in [
        (1_u16, 1_u16),
        (12, 5),
        (32, 10),
        (40, 12),
        (80, 24),
        (120, 40),
    ] {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();

        app.handle_key(key(KeyCode::Char('s')), mono, &clock)
            .unwrap();
        assert_eq!(app.overlay_kind(), Some(OverlayKind::Settings));
        app.render(&mut terminal).expect("render settings modal");

        select_field(&mut app, SettingsField::SignalActivityEnabled, mono, &clock);
        app.handle_key(key(KeyCode::Enter), mono, &clock).unwrap();
        assert_eq!(app.overlay_kind(), Some(OverlayKind::SignalConsent));
        app.render(&mut terminal)
            .expect("render signal consent modal");

        app.handle_key(key(KeyCode::Esc), mono, &clock).unwrap();
        assert_eq!(app.overlay_kind(), Some(OverlayKind::Settings));
        app.handle_key(key(KeyCode::Esc), mono, &clock).unwrap();
        assert_eq!(app.overlay_kind(), None);
    }
}
