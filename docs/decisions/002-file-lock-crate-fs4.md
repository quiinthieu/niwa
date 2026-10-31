# ADR 002: Select `fs4` Crate for Cross-Platform Advisory File Locks

- **Status:** Accepted
- **Date:** 2026-10-31
- **Context:** `PLAN.md` §3.3 ("`fs4` or `fd-lock` (file lock)") and T05 ("file lock ... Two concurrent processes don't corrupt the save")

## Context

`tg-store` is Niwa's sole filesystem writer (`PLAN.md` §3.1) and coordinates concurrent processes (`niwa`, `niwa daemon`, CLI actions) through an advisory exclusive file lock on `garden.lock` (`PLAN.md` §3.4 & §6).

`PLAN.md` §3.3 permits either `fs4` or `fd-lock`. Meanwhile, workspace lints forbid unsafe code (`unsafe_code = "forbid"` in `Cargo.toml`), and `GardenLock::acquire` / `GardenLock::try_acquire` should return an owned RAII lock guard (`GardenLockGuard`) without forcing callers to hold a separate `RwLock<File>` local binding across scopes.

## Decision

1. **Choose `fs4` (`default-features = false, features = ["sync"]`)** over `fd-lock`:
   - `fs4::fs_std::FileExt` (`lock_exclusive`, `try_lock_exclusive`, `unlock`) operates directly on an owned `std::fs::File` (backed by `rustix::fs::flock` on Unix), allowing `GardenLockGuard` to own the `File` with zero lifetime parameters and zero `unsafe` code in `tg-store`.
   - Disabling default features and enabling only `sync` avoids pulling in any async runtime integrations while keeping licenses strictly within `MIT OR Apache-2.0`.
2. **Locking Discipline:**
   - Always open `garden.lock` with `OpenOptions::new().create(true).read(true).write(true).truncate(false)` so acquiring or contending on the lock never truncates an existing inode.
   - Release the lock explicitly on `Drop` via `FileExt::unlock(&self.file)` (ignoring benign close errors) in addition to OS kernel cleanup on descriptor close / process exit.

## Consequences

- `tg-store` exposes an owned `GardenLock` RAII guard that prevents inter-process save races across macOS and Linux.
- `fd-lock` is not added to `Cargo.toml`.
