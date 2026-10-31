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
