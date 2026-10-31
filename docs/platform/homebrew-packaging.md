# Platform Verification: Release Packaging & Homebrew Tap (`quiinthieu/homebrew-tap`)

- **Verified on:** `2026-10-31`
- **Plan references:** `PLAN.md` §0.8, §1.1 (G6), §1.3, §8 & §10 T17
- **Single-source product constants:**
  - [`crates/tg-core/src/product.rs`](../../crates/tg-core/src/product.rs)
  - [`macos/Config/Product.xcconfig`](../../macos/Config/Product.xcconfig)

---

## 1. Re-Run of `PLAN.md` §1.3 Name Clash Check (`2026-10-31`)

Before publishing `quiinthieu/homebrew-tap` and `.github/workflows/release.yml`, every registry listed in `PLAN.md` §1.3 was re-queried live on `2026-10-31`:

| Registry / Endpoint | Queried URL | HTTP Status (`2026-10-31`) | Result |
|---|---|---:|---|
| Homebrew Core Formula (`niwa`) | `https://formulae.brew.sh/api/formula/niwa.json` | `404` | **Free** — no `niwa` formula in `homebrew/core` |
| Homebrew Core Formula (`niwa-garden`) | `https://formulae.brew.sh/api/formula/niwa-garden.json` | `404` | **Free** — no `niwa-garden` formula in `homebrew/core` |
| Homebrew Cask (`niwa`) | `https://formulae.brew.sh/api/cask/niwa.json` | `404` | **Free** — no `niwa` cask in `homebrew/cask` |
| Homebrew Cask (`niwa-garden`) | `https://formulae.brew.sh/api/cask/niwa-garden.json` | `404` | **Free** — no `niwa-garden` cask in `homebrew/cask` |
| crates.io (`niwa`) | `https://crates.io/api/v1/crates/niwa` | `200` | Taken (unrelated crate updated `2025-12-11`); we use **`niwa-garden`** (`[[bin]] name = "niwa"`) |
| crates.io (`niwa-garden`) | `https://crates.io/api/v1/crates/niwa-garden` | `404` | **Free** — available for `CLI_CRATE = "niwa-garden"` |
| npm (`niwa`) | `https://registry.npmjs.org/niwa` | `200` | Taken (`"NIWA intertwined web applications"`); separate JS ecosystem |
| npm (`niwa-garden`) | `https://registry.npmjs.org/niwa-garden` | `404` | **Free** (not publishing to npm) |
| PyPI (`niwa`) | `https://pypi.org/pypi/niwa/json` | `200` | Taken (`secemp9`'s Python CLI, which also installs a `niwa` executable); README (T18) notes the potential `PATH` order clash |
| PyPI (`niwa-garden`) | `https://pypi.org/pypi/niwa-garden/json` | `404` | **Free** (not publishing to PyPI) |

- **GitHub repositories checked (`2026-10-31`):** `secemp9/niwa` (Python planning CLI), `tsukumogami/niwa` (declarative workspace/dotfile tool), `okuramasafumi/niwa` (Ruby annotations), `OatsAndObjects/Niwa` (Android game), `dev-protocol/niwa`, and `theanhgen/niwaki` (Claude-hook forest). None publish a `niwa` or `niwa-garden` formula/cask to `formulae.brew.sh` or `niwa-garden` to `crates.io`.
- **Confirmed canonical identifiers:**
  - Display name: **`Niwa`** (`DISPLAY_NAME`)
  - CLI binary: **`niwa`** (`CLI_BIN`), Cargo crate: **`niwa-garden`** (`CLI_CRATE`)
  - Homebrew tap: **`quiinthieu/homebrew-tap`** (`brew tap quiinthieu/tap`)
  - Homebrew formula: **`niwa`** (`quiinthieu/tap/niwa`, `HOMEBREW_FORMULA`)
  - Homebrew cask: **`niwa-garden`** (`quiinthieu/tap/niwa-garden`, `HOMEBREW_CASK`)
  - macOS bundle identifier: **`io.github.quiinthieu.niwa`** (`BUNDLE_ID`)

---

## 2. Homebrew Cask `depends_on formula: "niwa"` Syntax & Stanza Order (`Homebrew 7.0.11`)

- **Verified against:**
  - Official Homebrew Cask Cookbook: `https://docs.brew.sh/Cask-Cookbook#stanza-depends_on`
  - Official Homebrew Tap Guide: `https://docs.brew.sh/How-to-Create-and-Maintain-a-Tap`
  - Local `Homebrew 7.0.11` implementation (`Library/Homebrew/cask/dsl/depends_on.rb`, `Library/Homebrew/utils/topological_hash.rb`, `Library/Homebrew/rubocops/cask/stanza_order.rb`, and `Library/Homebrew/cask/audit.rb`).
- **Verified `depends_on` DSL (`Cask::DSL::DependsOn`):**
  - `VALID_KEYS = Set[:formula, :cask, :macos, :maximum_macos, :linux, :arch]`.
  - Keyword argument syntax **`depends_on formula: "niwa"`** is canonical (`depends_on :formula` is rejected).
  - When a user installs `brew install --cask quiinthieu/tap/niwa-garden`, Homebrew taps `quiinthieu/homebrew-tap` first and resolves `depends_on formula: "niwa"` via `Formula["niwa"]` (`TopologicalHash.graph_package_dependencies`), finding `Formula/niwa.rb` in the same tap (`quiinthieu/tap`) since no `niwa` formula exists in `homebrew/core`.
- **Minimum macOS version & RuboCop stanza ordering (`brew audit --strict`):**
  - `Niwa.app` builds with `MACOSX_DEPLOYMENT_TARGET = 14.0` (`LSMinimumSystemVersion = 14.0`, macOS Sonoma).
  - `Cask::Audit#audit_min_os` and `Homebrew/OSDependsOn` (`Homebrew 7.0.11`) require casks targeting macOS 14+ to declare **`depends_on macos: :sonoma`** (replacing legacy `">= :sonoma"`).
  - `RuboCop::Cop::Cask::StanzaOrder#depends_on_sort_key` sorts consecutive `depends_on` stanzas alphabetically by symbol key (`"formula"` before `"macos"`), so the strict-audit-clean stanza order is:
    ```ruby
    depends_on formula: "niwa"
    depends_on macos: :sonoma
    ```

---

## 3. Release Artifacts, Universal `lipo` Binaries & Code Signing (`release.yml`)

- **Artifacts built by [`.github/workflows/release.yml`](../../.github/workflows/release.yml):**
  1. `niwa-v<VERSION>-universal-apple-darwin.tar.gz` — macOS universal Mach-O binary (`lipo -create` combining `aarch64-apple-darwin` and `x86_64-apple-darwin`, verified via `lipo -archs` -> `x86_64 arm64`), `LICENSE-MIT`, `LICENSE-APACHE`, and `packaging/launchd/io.github.quiinthieu.niwa.daemon.plist.in`.
  2. `niwa-v<VERSION>-x86_64-unknown-linux-gnu.tar.gz` — Linux `x86_64` binary, licenses, and `packaging/systemd/niwa.service.in`.
  3. `niwa-v<VERSION>-aarch64-unknown-linux-gnu.tar.gz` — Linux `arm64` binary (cross-compiled with `gcc-aarch64-linux-gnu`, verified as an `ARM aarch64` ELF binary, and smoke-tested under `qemu-aarch64`), licenses, and `packaging/systemd/niwa.service.in`.
  4. `Niwa-v<VERSION>-macos.dmg` — Compressed `UDZO` disk image (`hdiutil create -format UDZO`) containing universal (`arm64` + `x86_64`) `Niwa.app` and `/Applications` symlink.
  5. `SHA256SUMS.txt` — SHA-256 checksums covering all four release archives, verified with `shasum -a 256 -c SHA256SUMS.txt`.
- **Signing, Notarization & Zero-Cost Ad-Hoc Fallback:**
  - **When Apple Developer ID secrets are configured (`APPLE_CERTIFICATE_P12_BASE64`, `APPLE_CERTIFICATE_PASSWORD`, plus App Store Connect API Key or Apple ID notarization secrets):**
    - Imports the certificate into an ephemeral `$RUNNER_TEMP` keychain (destroyed in an `if: always()` cleanup step).
    - Signs `Niwa.app` with `codesign --force --deep --options runtime --timestamp --sign "$SIGNING_IDENTITY"`.
    - Submits to Apple Notary Service via `xcrun notarytool submit --wait`, staples tickets via `xcrun stapler staple` to both `Niwa.app` and `Niwa-v<VERSION>-macos.dmg`, and verifies Gatekeeper acceptance with `spctl -a -vvv -t exec Niwa.app`.
  - **When Apple Developer ID secrets are absent ($0 open-source mode):**
    - Signs `Niwa.app` ad-hoc (`codesign --force --deep --sign -`), verifies Mach-O and bundle signature integrity with `codesign --verify --deep --strict --verbose=2`, and mount-verifies the resulting `.dmg`.
    - Because `quiinthieu/homebrew-tap` is a third-party tap (`!cask.tap.official?`), `brew audit --strict` passes without paid Apple notarization (`Cask::Audit#audit_signing`).
    - `Casks/niwa-garden.rb` includes a `caveats` block documenting `brew install --cask --no-quarantine quiinthieu/tap/niwa-garden` and `xattr -dr com.apple.quarantine /Applications/Niwa.app` while preserving macOS's default Gatekeeper quarantine posture.
