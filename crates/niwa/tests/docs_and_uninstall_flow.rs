use assert_cmd::Command;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use niwa_garden::runtime::GardenSession;
use niwa_garden::tui::app::{OverlayKind, TuiApp};
use ratatui::backend::TestBackend;
use ratatui::style::{Color, Modifier};
use ratatui::Terminal;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tempfile::TempDir;
use tg_core::clock::{Clock, FakeClock, TICKS_PER_DAY};
use tg_core::product;
use tg_render::ColorTier;
use tg_store::paths::NiwaPaths;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/niwa sits two levels below workspace root")
        .to_path_buf()
}

#[derive(Debug, PartialEq, Eq)]
enum EntrySnapshot {
    Dir,
    File(Vec<u8>),
}

fn snapshot_tree(root: &Path) -> BTreeMap<PathBuf, EntrySnapshot> {
    fn walk(base: &Path, current: &Path, out: &mut BTreeMap<PathBuf, EntrySnapshot>) {
        let mut entries: Vec<_> = fs::read_dir(current)
            .unwrap_or_else(|err| panic!("failed reading dir {}: {err}", current.display()))
            .map(|res| res.expect("dir entry"))
            .collect();
        entries.sort_by_key(|e| e.file_name());

        for entry in entries {
            let path = entry.path();
            let rel = path
                .strip_prefix(base)
                .expect("entry under root")
                .to_path_buf();
            let file_type = entry.file_type().expect("entry file type");
            if file_type.is_dir() {
                out.insert(rel, EntrySnapshot::Dir);
                walk(base, &path, out);
            } else {
                let bytes = fs::read(&path)
                    .unwrap_or_else(|err| panic!("failed reading {}: {err}", path.display()));
                out.insert(rel, EntrySnapshot::File(bytes));
            }
        }
    }

    let mut map = BTreeMap::new();
    walk(root, root, &mut map);
    map
}

fn fresh_home_cmd(home: &Path) -> Command {
    let mut cmd = Command::cargo_bin(product::CLI_BIN).expect("niwa binary built");
    cmd.env_remove("NIWA_HOME")
        .env_remove("ZDOTDIR")
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("XDG_STATE_HOME", home.join(".local").join("state"))
        .env("SHELL", "/bin/zsh")
        .env("NO_COLOR", "1");
    cmd
}

fn xml_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Runs a deterministic multi-week `GardenSession` + `TuiApp` session on a `92x28`
/// `ratatui::backend::TestBackend` in `ColorTier::Truecolor` and serializes the exact
/// terminal cell buffer into an SVG screenshot.
fn render_real_tui_garden_svg(export_home: Option<&Path>) -> String {
    let tmp = TempDir::new().expect("create temp NIWA_HOME for real screenshot capture");
    let home_path = match export_home {
        Some(dir) => {
            fs::create_dir_all(dir).expect("create export_home dir");
            dir.to_path_buf()
        }
        None => tmp.path().to_path_buf(),
    };
    let paths = NiwaPaths::from_niwa_home(&home_path);

    // 2026-04-06T09:00:00Z -> Northern Spring.
    let clock = FakeClock::from_unix_secs(1_775_466_000);
    let mut session = GardenSession::open(paths.clone(), &clock, Some(0x4E49_5741_2026))
        .expect("open initial starter session");

    // Cycle 1 & 2: plant available seeds, wake halfway so 7-day dormancy never pauses growth,
    // let every plot bloom, and harvest all blooming plants for real seasonal drops.
    for _ in 0..2 {
        let seeds: Vec<String> = session
            .save()
            .inventory
            .iter()
            .filter(|(id, &cnt)| {
                cnt > 0
                    && session
                        .catalog()
                        .get(id)
                        .is_some_and(|sp| sp.can_sprout_in(session.current_season()))
            })
            .map(|(id, _)| id.clone())
            .collect();
        let mut plot_cursor = 0;
        for sp_id in seeds.iter() {
            while plot_cursor < session.garden().plots().len()
                && session.garden().plots()[plot_cursor].plant().is_some()
            {
                plot_cursor += 1;
            }
            if plot_cursor >= 6 {
                break;
            }
            let _ = session.plant(plot_cursor, sp_id, clock.now());
            plot_cursor += 1;
        }
        for _ in 0..3 {
            clock.advance_ticks(4 * TICKS_PER_DAY);
            session = GardenSession::open(paths.clone(), &clock, None).expect("catch up cycle");
            session
                .wake_on_tui_open(clock.now())
                .expect("record active check-in");
        }
        let _ = session.harvest_all(clock.now()).expect("harvest blooming");
    }

    // Plant 8 plots across staggered days so the live garden displays Blooming, Mature, Young,
    // Sprout, and 1 Empty plot simultaneously from real simulation ticks.
    let schedule: &[(usize, u64)] = &[
        (0, 4 * TICKS_PER_DAY),
        (1, 2 * TICKS_PER_DAY),
        (2, 2 * TICKS_PER_DAY),
        (3, TICKS_PER_DAY),
        (4, TICKS_PER_DAY),
        (5, 16 * 60),
        (6, 10 * 60),
        (7, 6 * 60),
    ];

    let mut chosen_species: Vec<String> = session
        .save()
        .inventory
        .iter()
        .filter(|(id, &cnt)| {
            cnt > 0
                && session
                    .catalog()
                    .get(id)
                    .is_some_and(|sp| sp.can_sprout_in(session.current_season()))
        })
        .map(|(id, _)| id.clone())
        .collect();
    // Sort rarest-first so Plot #1 highlights a discovered seasonal bloom.
    chosen_species.sort_by_key(|id| {
        std::cmp::Reverse(
            session
                .catalog()
                .get(id)
                .map(|sp| (sp.rarity, sp.id.clone())),
        )
    });

    let mut round_robin = 0usize;
    for &(plot_idx, advance_after) in schedule {
        if session.garden().plots()[plot_idx].plant().is_none() && !chosen_species.is_empty() {
            let mut pick = None;
            for offset in 0..chosen_species.len() {
                let candidate = &chosen_species[(round_robin + offset) % chosen_species.len()];
                if session
                    .save()
                    .inventory
                    .get(candidate.as_str())
                    .copied()
                    .unwrap_or(0)
                    > 0
                {
                    pick = Some(candidate.clone());
                    round_robin = (round_robin + offset + 1) % chosen_species.len();
                    break;
                }
            }
            if let Some(sp_id) = pick {
                session
                    .plant(plot_idx, &sp_id, clock.now())
                    .expect("plant scheduled plot");
            }
        }
        if advance_after > 0 {
            clock.advance_ticks(advance_after);
            session = GardenSession::open(paths.clone(), &clock, None)
                .expect("catch up staggered planting");
            session
                .wake_on_tui_open(clock.now())
                .expect("record active check-in during staggered planting");
        }
    }

    let mut app =
        TuiApp::new(session, ColorTier::Truecolor, &clock).expect("initialize Truecolor TuiApp");
    if app.overlay_kind() == Some(OverlayKind::AwaySummary) {
        app.handle_key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            Duration::ZERO,
            &clock,
        )
        .expect("dismiss away modal for main garden capture");
    }

    let mut terminal = Terminal::new(TestBackend::new(92, 28)).expect("create 92x28 TestBackend");
    app.render(&mut terminal)
        .expect("render real TUI garden frame");

    buffer_to_terminal_svg(terminal.backend().buffer())
}

fn buffer_to_terminal_svg(buf: &ratatui::buffer::Buffer) -> String {
    const CELL_W: f64 = 8.4;
    const CELL_H: f64 = 19.0;
    const PAD_X: f64 = 18.0;
    const HEADER_H: f64 = 38.0;
    const PAD_BOTTOM: f64 = 16.0;

    let cols = f64::from(buf.area.width);
    let rows = f64::from(buf.area.height);
    let total_w = (PAD_X * 2.0 + cols * CELL_W).round() as u32;
    let total_h = (HEADER_H + rows * CELL_H + PAD_BOTTOM).round() as u32;

    let mut svg = String::with_capacity(40 * 1024);
    let _ = writeln!(
        svg,
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {total_w} {total_h}" width="{total_w}" height="{total_h}" role="img" aria-label="Niwa interactive terminal garden screenshot">"##
    );
    let _ = writeln!(
        svg,
        r##"  <rect x="0.5" y="0.5" width="{}" height="{}" rx="11" fill="#151921" stroke="#2c3342"/>"##,
        total_w - 1,
        total_h - 1
    );
    let _ = writeln!(
        svg,
        r##"  <path d="M1 11.5C1 5.7 5.7 1 11.5 1h{}C{} 1 {} 5.7 {} 11.5V34H1V11.5z" fill="#1d222d"/>"##,
        total_w - 23,
        total_w - 6,
        total_w - 1,
        total_w - 1
    );
    let _ = writeln!(
        svg,
        r##"  <line x1="1" y1="34" x2="{}" y2="34" stroke="#293040"/>"##,
        total_w - 1
    );
    let _ = writeln!(
        svg,
        r##"  <circle cx="20" cy="17.5" r="5.5" fill="#ff5f56"/>
  <circle cx="38" cy="17.5" r="5.5" fill="#ffbd2e"/>
  <circle cx="56" cy="17.5" r="5.5" fill="#27c93f"/>"##
    );
    let _ = writeln!(
        svg,
        r##"  <g font-family="Menlo, Monaco, 'SF Mono', 'Cascadia Mono', Consolas, monospace" font-size="13.5">"##
    );
    let _ = writeln!(
        svg,
        r##"    <text x="{:.1}" y="22" text-anchor="middle" fill="#8b95a7">niwa — 92×28</text>"##,
        f64::from(total_w) / 2.0
    );

    #[derive(Clone, PartialEq, Eq)]
    struct CellStyle {
        fg: String,
        bold: bool,
    }

    for y in 0..buf.area.height {
        let baseline_y = HEADER_H + (f64::from(y) + 0.78) * CELL_H;
        let underline_y = HEADER_H + (f64::from(y) + 0.92) * CELL_H;

        let mut run_start_x: Option<u16> = None;
        let mut run_style: Option<CellStyle> = None;
        let mut run_text = String::new();
        let mut ul_run: Option<(u16, u16, String)> = None;

        let flush_run = |out: &mut String,
                         start_x: &mut Option<u16>,
                         style: &mut Option<CellStyle>,
                         text: &mut String| {
            if let (Some(sx), Some(st)) = (start_x.take(), style.take()) {
                let trimmed = text.trim_end_matches(' ');
                if !trimmed.is_empty() {
                    let x_px = PAD_X + f64::from(sx) * CELL_W;
                    let weight = if st.bold {
                        r#" font-weight="bold""#
                    } else {
                        ""
                    };
                    let space_attr = if trimmed.contains(' ') {
                        r#" xml:space="preserve""#
                    } else {
                        ""
                    };
                    let _ = writeln!(
                        out,
                        r#"    <text x="{x_px:.1}" y="{baseline_y:.1}" fill="{}"{weight}{space_attr}>{}</text>"#,
                        st.fg,
                        xml_escape(trimmed)
                    );
                }
                text.clear();
            }
        };

        let flush_ul = |out: &mut String, active: &mut Option<(u16, u16, String)>| {
            if let Some((sx, ex, color)) = active.take() {
                let ux1 = PAD_X + f64::from(sx) * CELL_W;
                let ux2 = PAD_X + f64::from(ex) * CELL_W;
                let _ = writeln!(
                    out,
                    r#"    <line x1="{ux1:.1}" y1="{underline_y:.1}" x2="{ux2:.1}" y2="{underline_y:.1}" stroke="{color}" stroke-width="1.2"/>"#
                );
            }
        };

        for x in 0..buf.area.width {
            let cell = &buf[(x, y)];
            let sym = cell.symbol();
            let mods = cell.modifier;
            let bold = mods.contains(Modifier::BOLD);
            let dim = mods.contains(Modifier::DIM);
            let underline = mods.contains(Modifier::UNDERLINED);

            let fg = match cell.fg {
                Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
                _ if dim => "#8490a3".to_string(),
                _ if bold => "#eef3f8".to_string(),
                _ => "#d5dde8".to_string(),
            };

            if underline && !sym.is_empty() {
                match ul_run {
                    Some((_, ref mut end_x, ref c)) if *end_x == x && *c == fg => {
                        *end_x = x + 1;
                    }
                    _ => {
                        flush_ul(&mut svg, &mut ul_run);
                        ul_run = Some((x, x + 1, fg.clone()));
                    }
                }
            } else {
                flush_ul(&mut svg, &mut ul_run);
            }

            if sym.is_empty() || sym == " " {
                if sym == " " && run_start_x.is_some() && !run_text.ends_with(' ') {
                    run_text.push(' ');
                } else {
                    flush_run(&mut svg, &mut run_start_x, &mut run_style, &mut run_text);
                }
                continue;
            }

            let cur_style = CellStyle { fg, bold };
            let is_ascii = sym.len() == 1 && sym.as_bytes()[0].is_ascii_graphic();

            if is_ascii {
                if run_style.as_ref() == Some(&cur_style) {
                    run_text.push_str(sym);
                } else {
                    flush_run(&mut svg, &mut run_start_x, &mut run_style, &mut run_text);
                    run_start_x = Some(x);
                    run_style = Some(cur_style);
                    run_text.push_str(sym);
                }
            } else {
                flush_run(&mut svg, &mut run_start_x, &mut run_style, &mut run_text);
                let x_px = PAD_X + f64::from(x) * CELL_W;
                let weight = if cur_style.bold {
                    r#" font-weight="bold""#
                } else {
                    ""
                };
                let _ = writeln!(
                    svg,
                    r#"    <text x="{x_px:.1}" y="{baseline_y:.1}" fill="{}"{weight}>{}</text>"#,
                    cur_style.fg,
                    xml_escape(sym)
                );
            }
        }

        flush_run(&mut svg, &mut run_start_x, &mut run_style, &mut run_text);
        flush_ul(&mut svg, &mut ul_run);
    }

    svg.push_str("  </g>\n</svg>\n");
    svg
}

#[test]
fn fresh_home_readme_workflow_install_use_and_uninstall_leaves_zero_residue() {
    let tmp = TempDir::new().expect("create isolated fresh HOME");
    let home = tmp.path();

    // Seed realistic pre-existing OS user directories and shell rc files.
    fs::create_dir_all(home.join("Library").join("Application Support"))
        .expect("seed ~/Library/Application Support");
    fs::create_dir_all(home.join(".config")).expect("seed ~/.config");
    fs::create_dir_all(home.join(".local").join("state")).expect("seed ~/.local/state");

    let initial_zshrc = b"export PATH=\"/opt/homebrew/bin:$PATH\"\nalias ll=\"ls -la\"\n";
    let initial_bashrc = b"# existing user bashrc\nexport EDITOR=vim\n";
    fs::write(home.join(".zshrc"), initial_zshrc).expect("seed .zshrc");
    fs::write(home.join(".bashrc"), initial_bashrc).expect("seed .bashrc");
    // Leave ~/.config/fish/config.fish absent before install so we verify created rc + dir cleanup too.

    let before = snapshot_tree(home);

    // 1. First run & inspection (`niwa status --json`, `niwa peek`).
    fresh_home_cmd(home)
        .args(["status", "--json"])
        .assert()
        .success();
    fresh_home_cmd(home)
        .args(["peek", "--width", "80", "--height", "24"])
        .assert()
        .success();

    // 2. Configuration (`niwa config set`).
    fresh_home_cmd(home)
        .args(["config", "set", "sprites", "ascii"])
        .assert()
        .success();
    fresh_home_cmd(home)
        .args(["config", "set", "sprites", "unicode"])
        .assert()
        .success();

    // 3. Opt-in signals & shell event ingestion.
    fresh_home_cmd(home)
        .args(["signals", "list"])
        .assert()
        .success();
    fresh_home_cmd(home)
        .args(["signals", "enable", "activity", "--yes"])
        .assert()
        .success();
    fresh_home_cmd(home)
        .args(["signals", "enable", "shell", "--yes"])
        .assert()
        .success();
    fresh_home_cmd(home)
        .args(["_ingest", "--exit", "0", "--dur", "3"])
        .assert()
        .success();

    // 4. Shell hook installation across zsh, bash, and fish (`--write-rc`).
    for shell in ["zsh", "bash", "fish"] {
        fresh_home_cmd(home)
            .args(["hook", "install", "--shell", shell, "--write-rc"])
            .assert()
            .success();
    }
    assert!(home.join(".zshrc.niwa.bak").is_file());
    assert!(home.join(".bashrc.niwa.bak").is_file());
    assert!(home
        .join(".config")
        .join("fish")
        .join("config.fish")
        .is_file());

    // 5. Background daemon unit install + single daemon tick (`garden.json.bak` created on 2nd save).
    fresh_home_cmd(home)
        .args(["daemon", "install"])
        .assert()
        .success();
    fresh_home_cmd(home)
        .args(["daemon", "status"])
        .assert()
        .success();
    fresh_home_cmd(home)
        .args(["daemon", "--once", "--idle-secs", "4"])
        .assert()
        .success();

    // 6. Ambient prompt fast path.
    fresh_home_cmd(home).arg("prompt").assert().success();
    fresh_home_cmd(home)
        .args(["prompt", "--style", "emoji"])
        .assert()
        .success();

    assert_ne!(
        snapshot_tree(home),
        before,
        "installed Niwa workflow must have populated state, config, hooks, and daemon unit"
    );

    // 7. Complete documented README uninstall sequence:
    //    `niwa hook uninstall` -> `niwa daemon uninstall` -> `niwa reset --all --yes`
    fresh_home_cmd(home)
        .args(["hook", "uninstall"])
        .assert()
        .success();
    fresh_home_cmd(home)
        .args(["daemon", "uninstall"])
        .assert()
        .success();
    fresh_home_cmd(home)
        .args(["reset", "--all", "--yes"])
        .assert()
        .success();

    let after = snapshot_tree(home);
    assert_eq!(
        after, before,
        "uninstall sequence left filesystem residue or altered pre-existing files in fresh HOME"
    );
}

#[test]
fn uninstall_preserves_unrelated_user_files_in_shared_directories() {
    let tmp = TempDir::new().expect("create isolated HOME with unrelated sibling files");
    let home = tmp.path();

    fs::create_dir_all(home.join("Library").join("LaunchAgents")).unwrap();
    fs::create_dir_all(home.join(".config").join("systemd").join("user")).unwrap();
    fs::create_dir_all(home.join(".config").join("fish").join("functions")).unwrap();

    let other_plist = home
        .join("Library")
        .join("LaunchAgents")
        .join("com.example.other.plist");
    let other_service = home
        .join(".config")
        .join("systemd")
        .join("user")
        .join("other.service");
    let other_fish_fn = home
        .join(".config")
        .join("fish")
        .join("functions")
        .join("greet.fish");
    fs::write(&other_plist, b"<plist/>").unwrap();
    fs::write(&other_service, b"[Unit]\n").unwrap();
    fs::write(&other_fish_fn, b"function greet; end\n").unwrap();

    fresh_home_cmd(home)
        .args(["hook", "install", "--shell", "fish", "--write-rc"])
        .assert()
        .success();
    fresh_home_cmd(home)
        .args(["daemon", "install"])
        .assert()
        .success();
    fresh_home_cmd(home)
        .args(["status", "--json"])
        .assert()
        .success();

    fresh_home_cmd(home)
        .args(["hook", "uninstall"])
        .assert()
        .success();
    fresh_home_cmd(home)
        .args(["daemon", "uninstall"])
        .assert()
        .success();
    fresh_home_cmd(home)
        .args(["reset", "--all", "--yes"])
        .assert()
        .success();

    assert!(other_plist.is_file(), "unrelated LaunchAgent must survive");
    assert!(
        other_service.is_file(),
        "unrelated systemd unit must survive"
    );
    assert!(
        other_fish_fn.is_file(),
        "unrelated fish function must survive"
    );
}

#[test]
fn v1_version_metadata_is_consistent_across_rust_macos_homebrew_plan_and_changelog() {
    let root = workspace_root();

    assert_eq!(
        product::VERSION,
        "1.0.0",
        "Cargo workspace and tg_core::product::VERSION must be bumped to 1.0.0 for T18"
    );

    let mut cmd = Command::cargo_bin(product::CLI_BIN).expect("niwa binary built");
    let out = cmd.arg("--version").output().expect("run niwa --version");
    assert!(out.status.success());
    let version_stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        version_stdout.contains("1.0.0"),
        "expected `niwa --version` to report 1.0.0, got: {version_stdout}"
    );

    let xcconfig = fs::read_to_string(root.join("macos/Config/Product.xcconfig"))
        .expect("read Product.xcconfig");
    assert!(
        xcconfig
            .lines()
            .any(|line| line.trim() == "NIWA_VERSION = 1.0.0"),
        "macos/Config/Product.xcconfig must set `NIWA_VERSION = 1.0.0`"
    );

    let formula = fs::read_to_string(
        root.join("packaging/homebrew/Formula")
            .join(format!("{}.rb", product::HOMEBREW_FORMULA)),
    )
    .expect("read Formula/niwa.rb");
    assert!(
        formula.contains("/refs/tags/v1.0.0.tar.gz"),
        "Formula/niwa.rb must point at v1.0.0 tag archive"
    );

    let cask = fs::read_to_string(
        root.join("packaging/homebrew/Casks")
            .join(format!("{}.rb", product::HOMEBREW_CASK)),
    )
    .expect("read Casks/niwa-garden.rb");
    assert!(
        cask.contains("version \"1.0.0\""),
        "Casks/niwa-garden.rb must specify version \"1.0.0\""
    );

    let plan = fs::read_to_string(root.join("PLAN.md")).expect("read PLAN.md");
    assert!(
        plan.contains("- [x] **T18 Docs & v1.0.**"),
        "PLAN.md §10 must tick T18 Docs & v1.0"
    );
    assert!(
        !plan.contains("- [ ] **T"),
        "all 18 v1 tasks in PLAN.md §10 must be checked off for v1.0.0"
    );

    let changelog = fs::read_to_string(root.join("CHANGELOG.md")).expect("read CHANGELOG.md");
    assert!(
        changelog.contains("## [1.0.0]"),
        "CHANGELOG.md must cut a ## [1.0.0] release section"
    );
    assert!(
        changelog.contains("T18 Docs & v1.0"),
        "CHANGELOG.md must record T18 Docs & v1.0"
    );
}

#[test]
fn export_real_screenshots_helper_when_requested() {
    let Some(export_dir) = std::env::var_os("NIWA_EXPORT_SCREENSHOT_DIR") else {
        return;
    };
    let export_dir = PathBuf::from(export_dir);
    fs::create_dir_all(&export_dir).expect("create NIWA_EXPORT_SCREENSHOT_DIR");
    let home_dir = export_dir.join("niwa_home");
    let _ = fs::remove_dir_all(&home_dir);
    let svg = render_real_tui_garden_svg(Some(&home_dir));
    fs::write(export_dir.join("tui-garden.svg"), svg).expect("write exported tui-garden.svg");
}

#[test]
fn readme_and_privacy_docs_cover_required_sections_real_screenshots_and_measured_numbers() {
    let root = workspace_root();
    let readme = fs::read_to_string(root.join("README.md")).expect("read README.md");
    let privacy = fs::read_to_string(root.join("docs/privacy.md")).expect("read docs/privacy.md");
    let baseline_raw = fs::read_to_string(root.join("tests/fixtures/perf-baseline.json"))
        .expect("read perf-baseline.json");
    let baseline: Value = serde_json::from_str(&baseline_raw).expect("parse perf-baseline.json");

    assert!(
        readme.lines().count() >= 120,
        "README.md must be a complete user & uninstall guide, not a 1-line stub"
    );

    // Real screenshot assets referenced by README.md must exist on disk, carry valid headers,
    // and `docs/assets/tui-garden.svg` must match the deterministic real TuiApp capture byte-for-byte.
    let expected_tui_svg = render_real_tui_garden_svg(None);
    let committed_tui_svg =
        fs::read_to_string(root.join("docs/assets/tui-garden.svg")).expect("read tui-garden.svg");
    assert_eq!(
        committed_tui_svg, expected_tui_svg,
        "docs/assets/tui-garden.svg must match the real deterministic TuiApp capture byte-for-byte"
    );

    for (asset_rel, is_svg) in [
        ("docs/assets/tui-garden.svg", true),
        ("docs/assets/macos-menubar.png", false),
    ] {
        assert!(
            readme.contains(asset_rel),
            "README.md must embed real screenshot asset `{asset_rel}`"
        );
        let asset_path = root.join(asset_rel);
        assert!(
            asset_path.is_file(),
            "referenced screenshot `{asset_rel}` must exist on disk"
        );
        let bytes = fs::read(&asset_path).expect("read screenshot bytes");
        assert!(
            bytes.len() > 1024,
            "screenshot `{asset_rel}` must contain real rendered output (got {} bytes)",
            bytes.len()
        );
        if is_svg {
            let text = String::from_utf8_lossy(&bytes);
            assert!(
                text.contains("<svg") && text.contains("Niwa"),
                "`{asset_rel}` must be a valid SVG rendered from a real Niwa garden frame"
            );
        } else {
            assert_eq!(
                &bytes[..8],
                b"\x89PNG\r\n\x1a\n",
                "`{asset_rel}` must be a valid PNG image"
            );
        }
    }

    // Required installation, usage, disambiguation (PLAN.md §1.3), privacy, and uninstall topics.
    for required in [
        "brew tap quiinthieu/tap",
        "quiinthieu/tap/niwa",
        "quiinthieu/tap/niwa-garden",
        "--no-quarantine",
        "xattr -dr com.apple.quarantine",
        "PyPI",
        "which -a niwa",
        "niwaki",
        "niwa peek",
        "niwa status",
        "niwa prompt",
        "niwa signals",
        "niwa hook install",
        "niwa hook uninstall",
        "niwa daemon install",
        "niwa daemon uninstall",
        "niwa reset --all",
        "docs/privacy.md",
        "docs/perf.md",
        "docs/snapshot-schema.md",
        "data/balance.toml",
        "tunable",
    ] {
        assert!(
            readme.contains(required),
            "README.md missing required T18 / PLAN §1.3 content `{required}`"
        );
    }

    // Cited performance numbers in README.md must match measured figures in perf-baseline.json
    // (PLAN.md §0.9 & §10 T18: never invent numbers).
    let ingest_p95 = format!(
        "{:.3} ms",
        baseline["baseline"]["rust"]["ingest"]["shell_enabled"]["p95_ms"]
            .as_f64()
            .expect("ingest p95")
    );
    let prompt_p95 = format!(
        "{:.3} ms",
        baseline["baseline"]["rust"]["prompt"]["glyph"]["p95_ms"]
            .as_f64()
            .expect("prompt p95")
    );
    let catchup_events_p95 = format!(
        "{:.3} ms",
        baseline["baseline"]["rust"]["catch_up"]["thirty_day_with_events"]["p95_ms"]
            .as_f64()
            .expect("catch_up p95")
    );
    let menubar_cpu = format!(
        "{:.4}%",
        baseline["baseline"]["macos_menubar"]["idle_cpu_pct"]
            .as_f64()
            .expect("menubar idle_cpu_pct")
    );

    for measured in [&ingest_p95, &prompt_p95, &catchup_events_p95, &menubar_cpu] {
        assert!(
            readme.contains(measured.as_str()),
            "README.md must cite real measured baseline figure `{measured}` from tests/fixtures/perf-baseline.json"
        );
    }

    // docs/privacy.md must document all signal schemas and the complete zero-residue uninstall commands.
    for expected_privacy in [
        "scripts/lint-privacy.sh",
        "events.jsonl",
        "niwa signals disable",
        "--purge",
        "niwa hook uninstall",
        "niwa daemon uninstall",
        "niwa reset --all",
    ] {
        assert!(
            privacy.contains(expected_privacy),
            "docs/privacy.md missing expected guidance `{expected_privacy}`"
        );
    }
}
