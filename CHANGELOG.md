# Changelog

All notable changes to Niwa will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- **T01 Workspace & tooling:** Cargo workspace with `tg-core`, `tg-signals`, `tg-store`, `tg-render`, and `niwa-garden` (`niwa` binary) crate stubs.
- Single-source-of-truth product metadata in `crates/tg-core/src/product.rs`, wired into `niwa --version`.
- Workspace dependency-graph and allowed-dependency tests (`crates/niwa/tests/dependency_graph.rs`).
- Zero-network privacy lint (`scripts/lint-privacy.sh`) with planted-import integration tests (`crates/niwa/tests/privacy_lint.rs`).
- Toolchain pinning (`rust-toolchain.toml`), `rustfmt.toml`, `clippy.toml`, `deny.toml`, and GitHub Actions CI (`.github/workflows/ci.yml`) for Linux and macOS.
- **T02 Test harness & injection:** `Clock` trait, `Timestamp`, and `FakeClock` in `crates/tg-core/src/clock.rs`, plus `SystemClock` in `crates/tg-store/src/clock.rs`.
- Portable seeded PCG64 PRNG factory (`rng_from_seed`, `rng_for_tick`, `rng_for_stream`) in `crates/tg-core/src/rng.rs` and `insta` golden snapshot testing setup.
- `TestEnv` harness (isolated temporary `NIWA_HOME`, `HOME`, and XDG directories), `NiwaPaths` (`crates/tg-store/src/paths.rs`), and save/spool fixture loaders (`crates/tg-store/src/fixtures.rs`, `tests/fixtures/`).
- `tg-core` purity lint (`scripts/lint-core-purity.sh`) enforced in CI with planted-violation tests (`crates/niwa/tests/core_purity_lint.rs`).
