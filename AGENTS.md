# AGENTS.md

This repo builds **Niwa** (a cozy idle garden for your terminal and menu bar), which lives in your terminal and the macOS menu bar and grows while you work. The CLI binary is **`niwa`** (crate `niwa-garden`). The Homebrew formula is `niwa`, the cask is `niwa-garden` and the bundle id is `io.github.quiinthieu.niwa`. Names are set in `crates/tg-core/src/product.rs` and `macos/Config/Product.xcconfig`. It's a Rust workspace (core, signals, store, render, and the `niwa` CLI/TUI) plus a thin Swift 6 SwiftUI menu bar app in `macos/`.

**The source of truth is [`PLAN.md`](./PLAN.md). Read §0 "Instructions for agents" before doing anything.**

Short version:
- Repo: `git@github.com:quiinthieu/niwa.git`. Branch from `main` as `task/TNN-slug`, one task per PR, Conventional Commits, squash-merge after CI passes (PLAN "Repository & workflow").
- Work on the lowest-numbered unchecked task in `PLAN.md` §10, and only that task. When its acceptance criteria pass, tick it and update `CHANGELOG.md`.
- Write tests first. Use `TestEnv` (temp `NIWA_HOME`) and `FakeClock`. Never touch the real `$HOME`.
- `tg-core` is pure: no I/O, no clock, no env. Only `tg-store` writes to disk. The shell hook edits an rc file only with `--write-rc`, inside a marked block that can be removed.
- Privacy: no network ever, and every signal is opt-in. Store only counts and timestamps, never command text, paths, repo names or commit messages. `scripts/lint-privacy.sh` enforces the no-network rule.
- Respect the power budget (PLAN §7). Never invent stats or performance numbers. Platform APIs marked *(verify)* get checked and recorded in `docs/platform/`.
- Commands: `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, `cargo test --workspace`, `cargo deny check`, `scripts/lint-privacy.sh`, and for the macOS app `xcodebuild -project macos/TerminalGarden.xcodeproj -scheme TerminalGarden test`.
