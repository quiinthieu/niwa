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
- **T03 Core simulation:** Tunable gameplay balance parameters in `data/balance.toml` validated by `Balance` and `GrowthProfile` in `crates/tg-core/src/balance.rs`.
- Pure domain model (`Stage`, `Plant`, `Plot`, `SimError` in `crates/tg-core/src/model.rs`) and deterministic `Garden` simulation (`tick()`, `tick_n()`, `simulate_ticks()`, dormancy, and `(garden_seed, tick_index)` per-tick/per-plot seeding in `crates/tg-core/src/sim.rs`).
- Unit tests for every stage transition and dormancy cycle (`crates/tg-core/tests/sim_stages_and_dormancy.rs`) plus `proptest` suites for batching invariance, monotonicity, and non-loss (`crates/tg-core/tests/sim_properties.rs`).
- **T04 Species, rarity, seasons:** ADR `docs/decisions/001-date-crate-time.md` selecting `#![no_std]` `time` for deterministic UTC calendar season math, plus `Season`, `Hemisphere`, `SeasonMode`, and `resolve_season` in `crates/tg-core/src/season.rs`.
- Span-validated `data/species/*.toml` loader (`SpeciesCatalog`, `Species`, `Rarity`, `ColorHint`, and `SpeciesLoadError` reporting exact file/line/column in `crates/tg-core/src/species.rs`) with 16 initial species across all 4 rarities and 4 seasons, plus season-aware `Garden::tick_with_catalog`.
- Tunable seasonal and seed drop-table weights in `data/balance.toml` and deterministic `DropTable` sampler (`crates/tg-core/src/drops.rs`) with golden `insta` snapshot and hemisphere boundary tests (`crates/tg-core/tests/{seasons.rs,species_and_drops.rs}`).
- **T05 Store:** ADR `docs/decisions/002-file-lock-crate-fs4.md` selecting `fs4` for cross-platform advisory exclusive locks on `garden.lock` (`GardenLock` in `crates/tg-store/src/lock.rs`), plus macOS/Linux/`NIWA_HOME` path resolution (`NiwaPaths::resolve` and `PlatformKind` in `crates/tg-store/src/paths.rs`).
- Crash-safe atomic writer (`atomic_write_bytes` with fault injection in `crates/tg-store/src/atomic.rs`), `garden.json` v1 persistence with `.bak` rotation and recovery notices (`GardenStore`, `GardenSave`, `RecoveryNotice` in `crates/tg-store/src/save.rs`), and step-by-step save migration runner with `v0 -> v1` sample (`MigrationRunner`, `MigrateV0ToV1` in `crates/tg-store/src/migrate.rs`).
- Validated `config.toml` reader/writer covering all `PLAN.md` §6 settings (`AppConfig`, `ConfigStore` in `crates/tg-store/src/config.rs`) and versioned `snapshot.json` writer/reader tolerating unknown fields (`Snapshot`, `write_snapshot`, `read_snapshot` in `crates/tg-store/src/snapshot.rs`), verified by fault-injection, `.bak` recovery, `insta` golden, and multi-process concurrency tests (`crates/tg-store/tests/`).
