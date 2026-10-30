# Niwa: Implementation Plan (v1)

*Niwa: a cozy idle garden for your terminal and menu bar.*

> **Names:** "Niwa" is the display name. The CLI binary is **`niwa`** (crate `niwa-garden`). The Homebrew formula is **`niwa`**, the menu bar cask is **`niwa-garden`** and the bundle id is **`io.github.quiinthieu.niwa`**. They're defined once in `crates/tg-core/src/product.rs` and mirrored in `macos/Config/Product.xcconfig`. **Don't hardcode the names anywhere else.** §1.3 has the name clash check.

Niwa is a cozy idle garden that lives in your terminal (TUI, prompt glyph, tmux status) and, on macOS, in the menu bar. It grows slowly on its own and faster while you work. The work signals are active time, git commits and shell commands. Every signal is **opt-in and local-only**. There are no accounts, no network and no guilt mechanics: plants never die from neglect, they just rest.

---

## Repository & workflow

- **Repo:** `git@github.com:quiinthieu/niwa.git`.
- **Bootstrap (first agent only):**
  ```sh
  git clone git@github.com:quiinthieu/<repo>.git && cd <repo>
  git checkout -b main
  # add PLAN.md, AGENTS.md (+ CLAUDE.md containing "@AGENTS.md"), .gitignore (Rust target/, Xcode DerivedData/, .build/, .DS_Store)
  git add . && git commit -m "chore: add implementation plan and agent instructions"
  git push -u origin main
  ```
  Then make `main` the default branch and protect it on GitHub (PR required, CI green).
- **Per task:** branch from the latest `main` as `task/TNN-short-slug` (e.g. `task/T03-core-sim`). One task per branch. Use Conventional Commits (`feat(core): …`, `fix(render): …`, `test:`, `chore:`, `docs:`). Open a PR titled `TNN: <task title>` with the acceptance criteria as a checklist. Squash-merge once CI passes, then delete the branch.
- **CI:** T01 adds `.github/workflows/ci.yml`. It runs `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test --workspace`, `cargo deny check` and `scripts/lint-privacy.sh` on `ubuntu-latest` and `macos-latest`. From T14 it also runs `xcodebuild test` for `macos/` on the macOS runner. T17 adds `release.yml` on `v*` tags.
- **Never** force-push `main`, and never commit secrets, signing certificates or notarization credentials. Those belong in GitHub Actions secrets.

## 0. Instructions for agents (read first)

1. **Work on one task at a time, in order.** Pick the lowest-numbered unchecked task in §10. Split an oversized task here first (`T05a`, `T05b`).
2. **Write tests first.** Every PR leaves `cargo test --workspace` green.
3. **Keep `tg-core` pure.** No I/O, no clock, no env: `Clock` and the seeded `Rng` are injected. That makes catch-up and the tests deterministic.
4. **Privacy is a hard rule.** No network, ever: no telemetry, update pings or crash reports. `scripts/lint-privacy.sh` fails CI on networking crates or APIs (`reqwest`, `hyper`, `ureq`, `std::net`, `URLSession`). Signals store **counts and timestamps only**, never command text, paths, commit messages, repo names or window titles.
5. **Signals are opt-in** (`niwa signals enable <name>`), and the consent timestamp is kept in `config.toml`. With none enabled, the garden still grows from time alone.
6. **Only `tg-store` writes to disk,** and only inside the app's data, state and config dirs. The one exception is the shell-hook installer: it edits an rc file only with `--write-rc`, adding a marked block with a backup. Tests use a temp `HOME` (`TestEnv`).
7. **Stay inside the power budget (§7).** Every timer needs a comment that justifies its interval.
8. **Platform APIs marked *(verify)* are assumptions.** Check the current docs and record the URL and date in `docs/platform/<topic>.md` first.
9. **Never invent numbers.** Balance values live in `data/*.toml` and are labelled tunable. Docs only cite measured figures.
10. **No new dependencies** beyond §3.3 without an ADR (`docs/decisions/NNN-*.md`). When a task is done, tick it in §10 and update `CHANGELOG.md`.

## 1. Goals, non-goals, landscape

### 1.1 Goals (v1)
- **G1 Idle garden:** plots, growth stages, seasons and rarity tiers. Offline progress is caught up deterministically.
- **G2 Work-linked growth (opt-in):** active time, git commits and shell commands add sun and water, with caps so the garden can't be farmed.
- **G3 Ambient surfaces:** the TUI (`niwa`), `niwa peek`, a prompt/tmux glyph (`niwa prompt`) and a macOS menu bar companion.
- **G4 Cozy:** no death, no streak punishment, no notifications by default. Neglected plants go *dormant* and wake up when you return.
- **G5 Cheap to run** (§7) and **G6 easy to install** with Homebrew (`quiinthieu/tap/niwa` formula, `quiinthieu/tap/niwa-garden` cask).

### 1.2 Non-goals (v1)
Accounts, sync, social features, purchases or gacha. AI-agent-specific hooks (v2). A Windows build (WSL is fine) or a Linux tray app. Productivity analytics: it isn't a time tracker.

### 1.3 Landscape & naming

**Similar projects (checked 2026-10-30):**
- **[cbonsai](https://gitlab.com/jallbrit/cbonsai):** a C/ncurses bonsai generator. It keeps no state; Niwa keeps a garden that persists and grows with your work.
- **[pipes.sh](https://pipeseroni.github.io/):** an animated pipes screensaver to watch. Niwa is a stateful idle game that sits idle when unwatched.
- **[flower-garden-cli](https://github.com/bdavidzhang/flower-garden-cli):** a Python terminal garden where you water flowers by hand, with weather and achievements. Niwa grows passively from opt-in work signals, catches up offline and has a menu bar companion.
- **[Honeytree](https://github.com/ndcorder/honeytree) / [niwaki](https://github.com/theanhgen/niwaki):** terminal forests where each Claude Code response plants a tree (through a Stop hook). Honeytree's variant syncs to the cloud, and niwaki wilts when a streak breaks. Niwa is agent-neutral (time, git, shell), local only and never penalises you.
- **[Claude Farmer](https://github.com/claude-farmer/claude-farmer):** an idle pixel farm driven by Claude Code, with GitHub OAuth, social features and gacha. Niwa has no login, no network and no social layer, and it's TUI-first.
- **[Pip](https://adoptpip.app/) / [FidgetPet](https://disquiet.io/products/fidgetpet):** macOS menu bar pixel pets that grow over time or as you use the Mac. Niwa is terminal and prompt first, with the menu bar as an optional companion, and adds git and shell signals.

**Name clash check (Homebrew API, GitHub, crates.io, npm and PyPI, 2026-10-30):**
- **Homebrew:** no `niwa` formula or cask (formulae.brew.sh API returns 404 for both). Free.
- **crates.io: `niwa` is taken** (an unrelated v0.1.0 crate from 2025-12, 49 downloads). We publish the CLI crate as **`niwa-garden`**, with `[[bin]] name = "niwa"`.
- **npm `niwa`** ("NIWA intertwined web applications") and **PyPI `niwa`** (secemp9's async spec/planning CLI, which also installs a `niwa` command) are taken. Those are other ecosystems and we don't publish there. Mention the possible PATH clash with the PyPI tool in the README.
- **GitHub:** small repos named niwa (`OatsAndObjects/Niwa` is an Android game, `dev-protocol/niwa` is a social token launcher, `secemp9/niwa` is the PyPI tool above). **Competitor:** `theanhgen/niwaki` has a different but close name. Our README should make clear we're a different project.
- **Chosen:** display name **Niwa**, binary **`niwa`**, Homebrew formula **`niwa`** (through our own tap, `quiinthieu/tap/niwa`), cask **`niwa-garden`** (it's distinct, and it avoids confusing the GUI cask with the CLI formula). Bundle id is **`io.github.quiinthieu.niwa`**. Re-check before T17.

---

## 2. Glossary
**Plot:** a grid cell. **Stage:** `Seed → Sprout → Young → Mature → Blooming`. **GP:** the growth points a plant accumulates per stage. **Sun / Water:** the growth inputs from active time / commits and commands. **Tick:** 60 s of garden time. **Catch-up:** simulating the ticks since `last_tick_at`. **Signal:** an opt-in work-event source. **Snapshot:** a small JSON file read by the prompt and the menu bar.

## 3. Architecture

**Stack decision: a Rust core, TUI and CLI, plus an optional native Swift menu bar companion.**
1. Rust gives one small static, cross-platform binary that starts fast enough for prompt and hook calls, and `ratatui` is a mature TUI stack. That covers macOS, Linux and WSL from one codebase.
2. The menu bar has to feel native (SwiftUI `MenuBarExtra`, idle-time APIs, notarisation), so it's a thin Swift app. It owns no game logic: it reads the snapshot and shells out to `niwa`.
3. The core simulation lives only in Rust, so the rules can't drift between surfaces. Swift and Rust talk through a versioned JSON contract instead of an FFI build.

### 3.1 Crates (Cargo workspace) and the macOS app
| Unit | Responsibility |
|---|---|
| `crates/tg-core` | Pure model and simulation: `Garden`, `Plant`, `Species`, `Season`, `tick()`, `catch_up()`, balance loading from `data/`. No I/O. |
| `crates/tg-signals` | Activity, git and shell-spool adapters, consent checks, caps → `WorkEvent`s. Read-only. |
| `crates/tg-store` | Paths, `config.toml`, `garden.json` (versioned and migrated), the `events.jsonl` spool, `snapshot.json`, file lock, atomic writes. **The only writer.** |
| `crates/tg-render` | `Garden` → frames (ASCII and Unicode sprites, color tiers, compact glyph). No terminal I/O. |
| `crates/niwa` | The binary: clap CLI, ratatui TUI, `daemon` loop, `hook` and `_ingest` subcommands. |
| `macos/TerminalGarden` | Swift 6 `MenuBarExtra` app (macOS 14+ *(verify)*). It reads `snapshot.json`, shows a glyph and a popover, and runs `niwa` for actions. |

### 3.2 Dependency graph
`niwa → {tg-core, tg-signals, tg-store, tg-render}` · `tg-signals → {tg-core, tg-store}` · `tg-store → tg-core` · `tg-render → tg-core` · `tg-core → (nothing internal)`. `DependencyGraphTests` (T01) checks this with `cargo metadata`.

### 3.3 Allowed third-party dependencies
`serde`, `serde_json`, `toml`, `clap`, `ratatui`, `crossterm`, `directories`, `rand` + `rand_pcg` (seeded), `thiserror`, `anyhow` (binary only), `fs4` or `fd-lock` (file lock), `time` or `jiff` (dates and seasons; pick one in an ADR). Dev only: `insta`, `proptest`, `assert_cmd`, `tempfile`. Swift: no third-party packages in v1.

### 3.4 Runtime flow
Every command runs `lock → load → catch_up(now, spool) → save → snapshot`, except `prompt`, which only reads a snapshot that is less than 60 s old. The optional `niwa daemon` (a launchd/systemd user agent, T10) does the same once per tick and samples idle time. Without it, catch-up on the next open must give an identical result. The shell hook only appends to the spool.

## 4. Game design (v1, all numbers tunable in `data/balance.toml`)

### 4.1 Growth
- Per tick, `GP = (base + sun + water) × species × season`. **base** is a wall-time trickle that is always on, so a garden with no signals still matures over days. Tune it toward "a common plant blooms within a work week". **sun** applies to active ticks: the activity signal, or else a tick with the TUI open or a shell event. **water** comes from commits and commands, with diminishing returns and caps per hour and per day. A test checks that a loop of `true` commands never beats honest use.
- **Dormancy instead of death:** after `dormant_after_days` (default 7) with no active ticks, plants pause and nothing is ever lost.
- **Catch-up:** simulates in bounded chunks. Offline ticks count fully up to `offline_cap` (default 72 h), then only `base × offline_tail_factor`. Spool events are applied at their timestamps. Timestamps from the future or before the save existed are dropped and counted in diagnostics.
- **Determinism:** the RNG is seeded from `(garden_seed, tick_index)`, so batching doesn't change the outcome. This is property-tested.

### 4.2 Plants, rarity, seasons
- Species live in `data/species/*.toml`: id, display name, rarity, stages, GP per stage, preferred seasons, sprites (ASCII and Unicode), color hint, and almanac flavour text.
- **Rarity tiers:** Common, Uncommon, Rare, Mythic. Seeds come from harvesting Blooming plants; the drop table is weighted by rarity and season. The weights sit in `balance.toml`. The UI never shows odds as "stats", only "rare" labels.
- **Seasons:** by default they follow the real calendar, with a `hemisphere` setting (north/south). The alternative is a `garden` mode that cycles every `season_length_days`. Seasons change growth multipliers, the palette and the ambient sprites (snow, petals, fireflies). Some species only sprout in their season.
- **Almanac:** the species you've discovered and their first-bloom dates. No streaks, no notifications.
- **Manual actions (TUI):** plant, harvest, compost (returns a seed of the same rarity), move. Nothing you click makes a plant grow.

### 4.3 Rendering
- **Sprite sets:** `unicode` (the default: box drawing, block elements and plant glyphs) and `ascii` (7-bit). No emoji in sprites because their width varies; they're allowed only in `prompt --style emoji`.
- **Color:** truecolor, 256 or 16, detected. Honours `NO_COLOR` and `--no-color`. Each season has its own palette.
- **Layout:** the grid fits the terminal size (minimum 40×12, with a compact list below that) and is tested for wide-character safety.
- **Animation:** at most 4 fps while focused, 0 fps when unfocused or idle for 30 s.
- **Compact glyph:** one cell (the stage of the most advanced plant) shared by the prompt, tmux and the menu bar title.

## 5. Work signals (all opt-in, local only)
| Signal | Source | Stored |
|---|---|---|
| `time` (always on) | wall clock | `last_tick_at` only |
| `activity` | system idle time. macOS: `CGEventSourceSecondsSinceLastEventType` *(verify)*, via the Swift app or a Rust binding. Linux: best effort (X11 screensaver ext / `org.freedesktop.ScreenSaver` *(verify)*) | `active: bool` per tick |
| `git` | configured repos, polled read-only (`git log --since --author=<email> --format=%ct`) every 5 min or on catch-up | commit timestamps only |
| `shell` | zsh `precmd`, bash `PROMPT_COMMAND`, fish `fish_postexec` *(verify)*, which run `niwa _ingest --exit $? --dur <s>` in the background | timestamp, ok/err class, duration bucket. **Never the command text** |

- **Consent:** `niwa signals` lists the signals. `enable <name>` shows exactly what will be stored and waits for `y`. `disable [--purge]` stops collection and can delete the events.
- **Spool:** `events.jsonl` is append-only and compacted after it's applied. Corrupt lines are skipped and counted.
- **`_ingest`:** append one line and exit, with no simulation and no lock wait. The latency target is in §7.
- **Uninstall:** `niwa hook uninstall` removes the rc block. `niwa reset --all` deletes data and config after confirmation.

## 6. Persistence & settings
- **Paths** (`directories`; `NIWA_HOME` overrides them in tests): macOS uses `~/Library/Application Support/Niwa/`. Linux uses `$XDG_STATE_HOME/niwa/` for data and `$XDG_CONFIG_HOME/niwa/config.toml` for config.
- **Files:** `garden.json` (`schema_version`, seed, plots, inventory, almanac, `last_tick_at`, diagnostics), `events.jsonl`, `snapshot.json` (versioned: glyph, summary, season, compact frame, `updated_at`), `config.toml`, `garden.lock`.
- **Safety:** temp file, then fsync, then atomic rename, with `garden.json.bak` kept. Migrations run one step at a time and are golden-tested. A corrupt save is restored from `.bak` with a notice and is never silently reset.
- **Settings** (`niwa config get|set`; the menu bar uses only this command): `sprites`, `color`, `fps`, `season_mode`, `hemisphere`, `signals.*` (with consent timestamp), `git.repos`, `git.author_email`, `dormant_after_days`, `prompt.style`, `menubar.show_title`.

## 7. Power & performance budget
These are **targets**, checked in T16. They aren't measured claims.
- **Daemon:** one coalesced wake per 60 s, no FS polling, and no daemon wakes at all when no signal is enabled.
- **TUI:** at most 4 fps, 0 fps when idle. **Menu bar app:** idle except for the 60 s timer and a `DispatchSource` watch on `snapshot.json`. The Instruments Energy Log should not flag it as high impact.
- **Latency:** p95 targets of under 10 ms for `_ingest` and under 15 ms for `prompt`. A 30-day catch-up takes under 200 ms in release on the CI runner.

## 8. Repo layout
`crates/{tg-core,tg-signals,tg-store,tg-render,niwa}/` · `data/{balance.toml,species/,sprites/{unicode,ascii}/}` · `macos/` (XcodeGen `Project.yml`, `Sources`, `Tests`, `Config/Product.xcconfig`) · `packaging/{homebrew,launchd,systemd}/` · `scripts/{lint-privacy.sh,bench.sh}` · `docs/{decisions,platform,privacy.md,perf.md,snapshot-schema.md}` · `tests/fixtures/{saves,spools}/` · root files `Cargo.toml`, `deny.toml`, `CHANGELOG.md`, `PLAN.md`, `AGENTS.md`, `CLAUDE.md`.

## 9. Test strategy
- **Unit (`tg-core`):** stages, caps, dormancy, seasons and hemispheres, drop table, all with `FakeClock` and a fixed seed.
- **Property (`proptest`):** catch-up batching invariance (`catch_up(N)` equals N ticks). GP is monotonic and time passing never deletes anything. Spam never exceeds the caps.
- **Golden (`insta`):** frames (sprite set × color tier × size), snapshot JSON, save migrations.
- **Integration (`assert_cmd`, `tempfile`):** CLI under a temp `NIWA_HOME`. Git against throwaway repos. Hook snippets run in real zsh, bash and fish in CI.
- **Privacy:** the lint, plus a scripted session followed by a grep of the save and spool for command text, paths and commit messages. The expected count is zero.
- **macOS:** XCTest for snapshot decoding (tolerating unknown fields) and the CLI wrapper (stub binary), plus a popover UI smoke test.
- **Perf:** `scripts/bench.sh` covers the §7 targets against a stored baseline. Docs cite only measured numbers.

## 10. Milestones & tasks
Each task fits in one agent session. Tick `[x]` when it's done. **M1 is the MVP**, a usable time-only garden in the terminal. Everything after it is additive.

### M0 Skeleton
- [x] **T01 Workspace & tooling.** Cargo workspace with the §3.1 crates as stubs, `product.rs`, `rust-toolchain.toml`, `deny.toml`, `ci.yml` (macOS + Linux), `scripts/lint-privacy.sh`, `DependencyGraphTests`, rustfmt and clippy config.
  *AC:* CI is green on both OSes. `--version` uses `product.rs`. The privacy lint catches a planted `std::net` import.
- [ ] **T02 Test harness & injection.** `Clock` trait (System/Fake), seeded `Rng` factory, `TestEnv` (temp `NIWA_HOME`), fixture loaders, insta setup.
  *AC:* a lint shows `tg-core` has no `std::fs`, `SystemTime::now` or `std::env`. The sample tests pass 100× in a loop.

### M1 MVP: time-only garden in the terminal
- [ ] **T03 Core simulation.** `Garden`, `Plot`, `Plant`, `Stage`, GP, `tick()`, dormancy, per-tick seeding.
  *AC:* unit tests for every stage transition and for dormancy. The batching-invariance and monotonicity properties pass.
- [ ] **T04 Species, rarity, seasons.** `data/species` loader with validation, an initial set of species across the four rarities (counts tunable; at least one per tier per season), drop table, calendar and garden season modes, hemisphere.
  *AC:* a bad species file gives a clear error with file and line. The season boundary tests pass for both hemispheres. Drop-table sampling over a fixed seed is golden.
- [ ] **T05 Store.** Paths, `garden.json` v1, atomic write + `.bak`, file lock, migration framework (a no-op v0→v1 sample), `config.toml` read/write, `snapshot.json` writer.
  *AC:* an aborted write leaves a loadable save. A corrupt save restores from `.bak` with a notice. Two concurrent processes don't corrupt the save.
- [ ] **T06 Offline catch-up.** `catch_up(now)` with `offline_cap`, the tail factor and clock-skew handling (a clock that moves backwards is ignored and noted). Builds a "While you were away" summary.
  *AC:* a 30-day catch-up gives the same result in one batch as in daily batches. The bench test stays under the §7 target. The summary is golden.
- [ ] **T07 Renderer.** Unicode and ASCII sprite sets for every species and stage, color tiers + `NO_COLOR`, grid layout + compact list fallback, compact glyph.
  *AC:* goldens at 40×12, 80×24 and 120×40 for each sprite set and color tier. No line overflows the width.
- [ ] **T08 TUI & CLI (MVP release).** `niwa` (ratatui): grid view, keyboard actions (plant, harvest, compost, move), almanac, help, quit. Plus `niwa peek`, `niwa status --json`, `niwa config`, `niwa reset`. The TUI counts as an active signal while open.
  *AC:* every subcommand has `assert_cmd` tests. A `TestBackend` script plants, fast-forwards and harvests. The fps cap and the idle drop to 0 fps hold. `v0.1.0` builds.

### M2 Work signals (opt-in)
- [ ] **T09 Signal framework & consent.** `WorkEvent`, spool format, `niwa signals list|enable|disable [--purge]`, consent records, per-hour and per-day caps with diminishing returns, diagnostics.
  *AC:* nothing is collected before `enable` (test). `--purge` deletes events. The spam property test passes. `docs/privacy.md` lists exactly what each signal stores.
- [ ] **T10 Activity signal + daemon.** `niwa daemon` (60 s loop), macOS idle time (an ADR decides between a Rust binding and the Swift app writing `activity` events), Linux best effort, `niwa daemon install|uninstall` for the launchd/systemd user units *(verify paths)*.
  *AC:* with the fake idle source, active ticks get the sun bonus. Install/uninstall round-trips in a temp home. The daemon skips its wake when no signal is enabled.
- [ ] **T11 Git signal.** `git.repos` and `git.author_email` config. Read-only `git log` polling stores timestamps only and dedupes across repos and re-polls.
  *AC:* temp-repo tests count only the configured author's new commits. Rewritten history doesn't double count. No repo path or message shows up in the save or spool (privacy test).
- [ ] **T12 Shell hook.** `niwa hook zsh|bash|fish` prints a snippet that backgrounds `niwa _ingest`. `hook install --write-rc` adds a marked block with a backup, and `hook uninstall` removes it. `_ingest` fast path.
  *AC:* real shells in CI produce spool lines with timestamp, exit class and duration bucket only. Install → uninstall restores the rc file byte for byte. The `_ingest` latency is measured and recorded.

### M3 Ambient surfaces & macOS menu bar
- [ ] **T13 Snapshot consumers.** `niwa prompt [--style glyph|emoji|none]` fast path, tmux status snippet, documented starship custom-module example, `docs/snapshot-schema.md` (versioned).
  *AC:* `prompt` never runs the simulation when the snapshot is fresh. Its latency is measured. Schema golden tests pass. Unknown fields are ignored.
- [ ] **T14 Swift menu bar app.** XcodeGen project, `MenuBarExtra` with the glyph title and a popover that renders the compact frame in a monospace font, a DispatchSource watch on `snapshot.json`, actions (harvest all, open TUI in Terminal, pause signals) that run `niwa`, and launch at login (`SMAppService` *(verify)*). If T10 chose the Swift route, it also hosts the idle-time sampler.
  *AC:* the XCTests and the UI smoke test pass. The app shows a helpful state when `niwa` is missing. The Instruments check is noted in `docs/perf.md`.
- [ ] **T15 Settings UI.** A menu bar Settings window and a TUI settings screen that cover every §6 key and write through `niwa config set`.
  *AC:* every `config.toml` key can be set from both the TUI and the menu bar app. Enabling a signal in the UI shows the same consent text as the CLI (shared strings file).

### M4 Polish & release
- [ ] **T16 Perf & content pass.** `bench.sh` for every §7 target with a stored baseline, a seasonal ambience pass, almanac flavour text, an accessibility check (ASCII mode, no color-only cues).
  *AC:* every §7 target has a measured value in `docs/perf.md` (or a filed issue if it's missed). There are no color-only state indicators.
- [ ] **T17 Packaging & Homebrew.** `release.yml`: macOS universal (`lipo` arm64 + x86_64) and Linux x86_64/arm64 tarballs with checksums. Signed and notarised `Niwa.app` DMG (secrets in Actions). Tap repo `quiinthieu/homebrew-tap` with formula `niwa` and cask `niwa-garden` (the cask `depends_on formula: "niwa"` *(verify syntax)*). Re-run the §1.3 name check first.
  *AC:* `brew install --build-from-source` and `brew audit --strict` pass in CI. A tag dry run produces every artifact. `spctl -a` accepts the app.
- [ ] **T18 Docs & v1.0.** README (screenshots from real runs, no invented numbers), `docs/privacy.md`, uninstall guide (`hook uninstall`, `daemon uninstall`, `reset --all`), CHANGELOG, and `v1.0.0`.
  *AC:* on a fresh macOS VM, the README works from install to uninstall and leaves no residue.

## 11. Risks & mitigations
- *Signals feel like surveillance:* opt-in, counts only, a privacy lint and test, `--purge`. *Grinding:* caps, no streaks, no penalties. *Prompt slowdown:* `_ingest` only appends, in the background, and its latency is measured. *Platform API changes:* the *(verify)* docs, isolated behind traits. *Rust↔Swift drift:* no game logic in Swift, plus snapshot schema goldens. *Name confusion:* `niwa` and our own tap.

## 12. v2 backlog (don't implement in v1)
AI-agent hooks (Claude Code/Codex) as an extra signal, text "postcards" to share, a Linux tray app, Windows, more biomes, multiple local profiles, folder-based sync.
