# ADR 001: Select `time` Crate for Calendar Dates and Seasons

- **Status:** Accepted
- **Date:** 2026-10-31
- **Context:** `PLAN.md` §3.3 ("`time` or `jiff` (dates and seasons; pick one in an ADR)") and T04 ("calendar and garden season modes, hemisphere")

## Context

`tg-core` needs deterministic civil date conversion (`Timestamp` Unix seconds → UTC year/month/day) to resolve real-calendar seasons across Northern and Southern hemispheres, while preserving `tg-core`'s strict purity contract (no filesystem I/O, no ambient wall clock, no runtime environment access).

`PLAN.md` §3.3 allows either `time` or `jiff` and asks that an ADR select one.

## Decision

1. **Choose `time` (`version = "0.3", default-features = false`)** over `jiff`:
   - With `default-features = false` (omitting `std` and `local-offset`), `time` is `#![no_std]`, pulls no embedded timezone database, performs zero environment or `/etc/localtime` I/O, exposes no `OffsetDateTime::now_utc()` wall-clock constructor, and adds only four tiny pure-arithmetic helper crates (`time-core`, `deranged`, `num-conv`, `powerfmt`), all under `MIT OR Apache-2.0`.
   - `time::OffsetDateTime::from_unix_timestamp(ts.as_unix_secs())` converts Unix timestamps to proleptic Gregorian UTC `(year, month, day)` deterministically across macOS, Linux, and all host `TZ` settings.
2. **Meteorological UTC Month Boundaries for Calendar Seasons:**
   - **Northern Hemisphere:** Spring = Mar–May, Summer = Jun–Aug, Autumn = Sep–Nov, Winter = Dec–Feb.
   - **Southern Hemisphere:** Autumn = Mar–May, Winter = Jun–Aug, Spring = Sep–Nov, Summer = Dec–Feb (exact two-season inversion).
   - Meteorological month starts (`Mar 1`, `Jun 1`, `Sep 1`, `Dec 1` at `00:00:00Z`) avoid year-to-year astronomical equinox drift and keep leap-year transitions (`Feb 28/29 23:59:59Z → Mar 1 00:00:00Z`) unambiguous and testable.

## Consequences

- `tg-core` remains 100% pure and passes `scripts/lint-core-purity.sh`, `scripts/lint-privacy.sh`, `cargo deny check`, and `DependencyGraphTests`.
- `jiff` is not added to `Cargo.toml`.
