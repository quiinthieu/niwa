# `snapshot.json` Schema & Ambient Consumer Contract (`schema_version = 1`)

`snapshot.json` is Niwa's small, versioned, read-only JSON surface written by `tg-store` whenever garden state advances (`niwa`, `niwa peek`, `niwa status`, `niwa daemon`, or a stale-snapshot refresh) and read by ambient surfaces:

- **`niwa prompt [--style glyph|emoji|none]`** (shell prompts, `tmux` status lines, and Starship custom modules)
- **`Niwa.app` (`macos/TerminalGarden`)** SwiftUI `MenuBarExtra` companion (watching `snapshot.json` via `DispatchSource`)

All game simulation rules live exclusively in Rust (`tg-core`). Consumers treat `snapshot.json` as an immutable projection and never write to it directly (`PLAN.md` §0.6 & §3).

---

## 1. File Location & Atomic Write Guarantee

| Platform | Resolved Path (`NiwaPaths::snapshot_file()`) |
|---|---|
| `NIWA_HOME` override (tests / custom roots) | `$NIWA_HOME/snapshot.json` |
| macOS | `~/Library/Application Support/Niwa/snapshot.json` |
| Linux / WSL (XDG) | `${XDG_STATE_HOME:-~/.local/state}/niwa/snapshot.json` |

### Atomic Replacement
`tg_store::snapshot::write_snapshot` serializes UTF-8 pretty JSON terminated by `\n`, writes and `fsync`s a sibling temporary file in the same directory, atomically renames it over `snapshot.json`, and syncs the parent directory. Consumers (`niwa prompt`, `DispatchSource`, `jq`) never observe a half-written or truncated JSON document.

---

## 2. Versioning & Forward-Compatibility Policy

- **Current schema version:** `schema_version = 1` (`tg_store::snapshot::CURRENT_SNAPSHOT_SCHEMA_VERSION`).
- **Additive minor fields (ignored when unknown):** New optional fields may be added under `schema_version = 1`. Every compliant consumer (Rust `serde` `Snapshot` without `deny_unknown_fields`, Swift `Codable` `Snapshot`, and user scripts) **must ignore unknown JSON keys** (`PLAN.md` §9, §10 T13 & §11).
- **Optional v1 fields with defaults:** `dormant` (`false`), `planted_count` (`0`), and `blooming_count` (`0`) default cleanly if omitted.
- **Invalid or missing `schema_version`:** Rejected as `StoreError::CorruptSave` (`schema_version` must be an integer `>= 1`).
- **Breaking future major versions (`schema_version > 1`):** Rejected explicitly as `StoreError::UnsupportedFutureSchema { kind: "snapshot.json", found, max_supported: 1 }` rather than silently misinterpreting incompatible state.

---

## 3. Freshness Window (`< 60 s`)

One garden tick is `60` seconds (`tg_core::clock::TICK_SECS = 60`, `tg_store::snapshot::SNAPSHOT_MAX_AGE_SECS = 60`).

- **Fresh (`0 <= now - updated_at < 60` seconds, or backward clock skew `now < updated_at`):**
  `niwa prompt` takes its lock-free read-only fast path (`PLAN.md` §3.4 & §10 T13). It reads only `config.toml` (when present) and `snapshot.json`, **never** acquires `garden.lock`, **never** reads `garden.json` or `events.jsonl`, and **never** runs `catch_up` or simulation.
- **Stale (`now - updated_at >= 60` seconds) or missing:**
  `niwa prompt` attempts a non-blocking `GardenLock::try_acquire`. If acquired, it runs `load → catch_up(now, spool) → save → snapshot` (throttling any opt-in `git` polling to the 5-minute periodic window) so daemon-less time-only gardens advance once per minute. If another process (such as an open interactive `niwa` TUI) currently holds `garden.lock`, `niwa prompt` never blocks the user's shell prompt and renders immediately from the existing snapshot.

---

## 4. Field Reference (`schema_version = 1`)

| Field | JSON Type | Required in v1 | Default if Omitted | Semantics |
|---|---|---|---|---|
| `schema_version` | integer (`u32`) | Yes | *(error if missing / `< 1`)* | Format major version. Always `1` for v1 snapshots. |
| `glyph` | string | Yes | — | Single-cell (1-column, emoji-free) glyph of the most advanced living plant in the garden, formatted for the active `sprites` set (`unicode` or `ascii`). |
| `summary` | string | Yes | — | Single-line human-readable status summary, e.g. `"4/8 planted · 1 blooming"` or `"2/8 planted · 0 blooming · resting"` when dormant. |
| `season` | string enum | Yes | — | Active garden season: `"spring"`, `"summer"`, `"autumn"`, or `"winter"`. |
| `compact_frame` | string | Yes | — | Plain-text (ANSI-free, emoji-free) multi-line monospace summary frame rendered for compact popovers such as the macOS `MenuBarExtra`. |
| `updated_at` | integer (`i64`) | Yes | — | Unix epoch timestamp in seconds (UTC) when the snapshot was written. |
| `dormant` | boolean | Optional | `false` | `true` when the garden is resting after `dormant_after_days` without active ticks (`PLAN.md` §4.1). |
| `planted_count` | integer (`usize`) | Optional | `0` | Number of plots currently occupied by a plant (`0..=total_plots`). |
| `blooming_count` | integer (`usize`) | Optional | `0` | Number of occupied plots currently at `Stage::Blooming` (`0..=planted_count`). |

### Canonical Stage Glyph & Prompt Emoji Mapping (`PLAN.md` §4.3)

Emoji are forbidden in `snapshot.json`'s `glyph` and `compact_frame` so cell widths remain deterministic across terminals; emoji are emitted **only** when a consumer selects `niwa prompt --style emoji` (or `prompt.style = "emoji"` in `config.toml`):

| Most Advanced Plant Stage | `glyph` (`sprites = "unicode"`) | `glyph` (`sprites = "ascii"`) | `niwa prompt --style emoji` |
|---|---|---|---|
| Empty garden (`None`) | `◌` | `_` | `🪴` |
| `Seed` | `·` | `.` | `🌰` |
| `Sprout` | `╷` | `,` | `🌱` |
| `Young` | `┼` | `+` | `🌿` |
| `Mature` | `◎` | `o` | `🌷` |
| `Blooming` | `◈` | `*` | `🌸` |

Because all 12 canonical Unicode and ASCII stage glyphs are pairwise distinct, `tg_render::glyph::stage_from_glyph` inverts `snapshot.glyph` back to `Option<Stage>` in `O(1)` without extra snapshot fields, allowing `niwa prompt` to switch seamlessly between `--style glyph`, `--style emoji`, and `sprites = ascii|unicode` on an already-fresh snapshot.

---

## 5. Golden Example (`schema_version = 1`)

```json
{
  "schema_version": 1,
  "glyph": "◈",
  "summary": "4/8 planted · 1 blooming",
  "season": "summer",
  "compact_frame": "Niwa · Summer · 4/8 planted · 1 blooming\n  1. ◈ Moss (Blooming)\n  2. ◎ Fern (Mature)",
  "updated_at": 1752000060,
  "dormant": false,
  "planted_count": 4,
  "blooming_count": 1
}
```

Golden regression snapshots for empty, active blooming, dormant winter, and 7-bit ASCII states live under `crates/tg-store/tests/snapshots/snapshot_schema__*.snap` and `paths_config_snapshot__snapshot_json_v1_golden.snap`.

---

## 6. Ambient Consumers

### 6.1 `niwa prompt`

```sh
niwa prompt                  # Uses `prompt.style` from config.toml (default: `glyph`)
niwa prompt --style glyph    # 1-cell glyph (`◈` in unicode mode, `*` in ascii mode)
niwa prompt --style emoji    # Stage emoji (`🪴`, `🌰`, `🌱`, `🌿`, `🌷`, `🌸`)
niwa prompt --style none     # Emits 0 bytes (cleanly hides prompt indicator)
```

Persist a default style across shells without editing prompt configs:

```sh
niwa config set prompt.style emoji
niwa config set prompt.style none
niwa config set prompt.style glyph
```

### 6.2 `tmux` Status-Bar Snippet (`niwa prompt --tmux`)

Print the snippet directly with `niwa prompt --tmux` (or `niwa prompt --tmux --style emoji`) and add it to `~/.tmux.conf`:

```tmux
# Niwa garden status glyph (60 s interval matches Niwa's 60 s garden tick budget)
set -g status-interval 60
set -ag status-right " #(niwa prompt)"
```

Setting `status-interval 60` aligns `tmux` refreshes with Niwa's 60-second tick cadence (`PLAN.md` §7) so `tmux` wakes at most once per minute.

### 6.3 Starship Custom Module (`niwa prompt --starship`)

Print the snippet directly with `niwa prompt --starship` (or `niwa prompt --starship --style emoji`) and add it to `~/.config/starship.toml`:

```toml
# Niwa idle garden glyph (reads fresh <60 s snapshot.json in a few milliseconds)
[custom.niwa]
command = "niwa prompt"
when = true
format = "[$output ]($style)"
style = "green"
ignore_timeout = true
description = "Niwa terminal garden stage indicator"
```

Placing the trailing space inside `[$output ]($style)` ensures Starship omits the entire group—including the trailing space—whenever `niwa config set prompt.style none` is active (`$output` is empty).

### 6.4 Direct Shell Prompt (`zsh` / `bash` / `fish`)

- **`zsh`** (`~/.zshrc`):
  ```zsh
  setopt PROMPT_SUBST
  PROMPT='$(niwa prompt) %~ %# '
  ```
- **`bash`** (`~/.bashrc`):
  ```bash
  PS1='$(niwa prompt) \w \$ '
  ```
- **`fish`** (`~/.config/fish/functions/fish_prompt.fish`):
  ```fish
  set -l garden (niwa prompt)
  test -n "$garden"; and echo -n "$garden "
  ```
