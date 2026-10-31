use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;
use tg_core::product;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/niwa sits two levels below workspace root")
        .to_path_buf()
}

#[test]
fn packaging_files_and_platform_verification_exist_and_are_bash32_safe() {
    let root = workspace_root();

    for rel in [
        "LICENSE-MIT",
        "LICENSE-APACHE",
        "docs/platform/homebrew-packaging.md",
        "scripts/render-homebrew.sh",
        "scripts/package-macos-dmg.sh",
        "scripts/verify-homebrew-ci.sh",
        "packaging/homebrew/Formula/niwa.rb",
        "packaging/homebrew/Casks/niwa-garden.rb",
        ".github/workflows/release.yml",
        ".github/workflows/ci.yml",
    ] {
        let path = root.join(rel);
        assert!(path.is_file(), "expected required T17 file {rel} to exist");
        let content =
            fs::read_to_string(&path).unwrap_or_else(|err| panic!("failed reading {rel}: {err}"));
        assert!(!content.trim().is_empty(), "{rel} must not be empty");
    }

    let platform_doc =
        fs::read_to_string(root.join("docs/platform/homebrew-packaging.md")).unwrap();
    for expected in [
        "2026-10-31",
        "https://formulae.brew.sh/api/formula/niwa.json",
        "https://formulae.brew.sh/api/cask/niwa-garden.json",
        "https://crates.io/api/v1/crates/niwa-garden",
        "https://registry.npmjs.org/niwa",
        "https://pypi.org/pypi/niwa/json",
        "depends_on formula: \"niwa\"",
        "depends_on macos: :sonoma",
    ] {
        assert!(
            platform_doc.contains(expected),
            "docs/platform/homebrew-packaging.md missing expected verification evidence: {expected}"
        );
    }

    for script_rel in [
        "scripts/render-homebrew.sh",
        "scripts/package-macos-dmg.sh",
        "scripts/verify-homebrew-ci.sh",
    ] {
        let script_path = root.join(script_rel);
        let script_body = fs::read_to_string(&script_path).unwrap();
        assert!(
            !script_body.contains("[@]"),
            "{script_rel} must not use Bash array [@] expansion under `set -u` (macOS /bin/bash 3.2 treats empty arrays as unbound)"
        );

        let status = Command::new("bash")
            .arg("-n")
            .arg(&script_path)
            .status()
            .unwrap_or_else(|err| panic!("failed to run `bash -n {script_rel}`: {err}"));
        assert!(
            status.success(),
            "`bash -n {script_rel}` failed with status {status}"
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&script_path).unwrap().permissions().mode();
            assert_ne!(
                mode & 0o111,
                0,
                "{script_rel} must have executable permission bits set (mode={mode:o})"
            );
        }
    }
}

#[test]
fn render_homebrew_derives_product_names_from_single_source_and_matches_committed_files() {
    let root = workspace_root();
    let script_path = root.join("scripts/render-homebrew.sh");
    let script_source = fs::read_to_string(&script_path).unwrap();

    // PLAN.md line 5: product names live only in crates/tg-core/src/product.rs and
    // macos/Config/Product.xcconfig. The generator must parse product.rs rather than
    // duplicating hardcoded literals.
    for forbidden_literal in [
        "\"niwa-garden\"",
        "\"io.github.quiinthieu.niwa\"",
        "'niwa-garden'",
        "'io.github.quiinthieu.niwa'",
    ] {
        assert!(
            !script_source.contains(forbidden_literal),
            "scripts/render-homebrew.sh must derive product identifiers from product.rs instead of hardcoding {forbidden_literal}"
        );
    }

    let tmp = TempDir::new().expect("create temp dir for rendered homebrew files");
    let output = Command::new("bash")
        .arg(&script_path)
        .arg("--output-dir")
        .arg(tmp.path())
        .output()
        .expect("execute scripts/render-homebrew.sh");
    assert!(
        output.status.success(),
        "render-homebrew.sh failed: stdout={}, stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let generated_formula_path = tmp
        .path()
        .join("Formula")
        .join(format!("{}.rb", product::HOMEBREW_FORMULA));
    let generated_cask_path = tmp
        .path()
        .join("Casks")
        .join(format!("{}.rb", product::HOMEBREW_CASK));

    let generated_formula = fs::read_to_string(&generated_formula_path)
        .expect("generated Formula/<HOMEBREW_FORMULA>.rb should exist");
    let generated_cask = fs::read_to_string(&generated_cask_path)
        .expect("generated Casks/<HOMEBREW_CASK>.rb should exist");

    let committed_formula = fs::read_to_string(
        root.join("packaging/homebrew/Formula")
            .join(format!("{}.rb", product::HOMEBREW_FORMULA)),
    )
    .unwrap();
    let committed_cask = fs::read_to_string(
        root.join("packaging/homebrew/Casks")
            .join(format!("{}.rb", product::HOMEBREW_CASK)),
    )
    .unwrap();

    assert_eq!(
        generated_formula,
        committed_formula,
        "committed packaging/homebrew/Formula/{}.rb is out of sync with scripts/render-homebrew.sh",
        product::HOMEBREW_FORMULA
    );
    assert_eq!(
        generated_cask,
        committed_cask,
        "committed packaging/homebrew/Casks/{}.rb is out of sync with scripts/render-homebrew.sh",
        product::HOMEBREW_CASK
    );

    // Tag archive URLs must omit redundant `version "..."` in Formula so Homebrew's
    // ResourceAuditor#audit_version passes under `brew audit --strict`.
    assert!(
        !generated_formula.contains(&format!("version \"{}\"", product::VERSION)),
        "tag-archive Formula must not emit a redundant `version` stanza"
    );

    // Commit archive URLs (pre-tag tap bootstrap) DO require an explicit `version "..."`
    // because Homebrew otherwise scans the 40-hex commit SHA as the version.
    let commit_tmp = TempDir::new().expect("create temp dir for commit-archive formula");
    let commit_sha = "eff016f0123456789abcdef0123456789abcdef0";
    let dummy_sha256 = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let commit_out = Command::new("bash")
        .arg(&script_path)
        .arg("--output-dir")
        .arg(commit_tmp.path())
        .arg("--formula-url")
        .arg(format!(
            "https://github.com/quiinthieu/niwa/archive/{commit_sha}.tar.gz"
        ))
        .arg("--formula-sha256")
        .arg(dummy_sha256)
        .output()
        .expect("execute scripts/render-homebrew.sh with commit archive URL");
    assert!(
        commit_out.status.success(),
        "commit-url render failed: {}",
        String::from_utf8_lossy(&commit_out.stderr)
    );
    let commit_formula = fs::read_to_string(
        commit_tmp
            .path()
            .join("Formula")
            .join(format!("{}.rb", product::HOMEBREW_FORMULA)),
    )
    .unwrap();
    assert!(
        commit_formula.contains(&format!("version \"{}\"", product::VERSION)),
        "commit-archive Formula must include explicit `version \"{}\"`",
        product::VERSION
    );
}

#[test]
fn homebrew_formula_and_cask_satisfy_t17_stanza_and_quarantine_contracts() {
    let root = workspace_root();
    let formula = fs::read_to_string(
        root.join("packaging/homebrew/Formula")
            .join(format!("{}.rb", product::HOMEBREW_FORMULA)),
    )
    .unwrap();
    let cask = fs::read_to_string(
        root.join("packaging/homebrew/Casks")
            .join(format!("{}.rb", product::HOMEBREW_CASK)),
    )
    .unwrap();

    // Formula checks
    for expected in [
        "class Niwa < Formula",
        "homepage \"https://github.com/quiinthieu/niwa\"",
        "license any_of: [\"MIT\", \"Apache-2.0\"]",
        "depends_on \"rust\" => :build",
        "std_cargo_args(path: \"crates/niwa\")",
        "ENV[\"NIWA_HOME\"] = (testpath/\"niwa-home\").to_s",
        &format!("bin/\"{}\"", product::CLI_BIN),
    ] {
        assert!(
            formula.contains(expected),
            "Formula/{}.rb missing expected snippet: {expected}",
            product::HOMEBREW_FORMULA
        );
    }

    // Cask checks
    let empty_sha256 = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    assert!(
        !cask.contains(empty_sha256),
        "Cask must never use the empty-string SHA-256 rejected by Cask::Audit#audit_sha256_invalid"
    );

    let dep_formula = format!("depends_on formula: \"{}\"", product::HOMEBREW_FORMULA);
    let dep_macos = "depends_on macos: :sonoma";
    let idx_formula = cask
        .find(&dep_formula)
        .unwrap_or_else(|| panic!("Cask missing `{dep_formula}`"));
    let idx_macos = cask
        .find(dep_macos)
        .unwrap_or_else(|| panic!("Cask missing `{dep_macos}`"));
    assert!(
        idx_formula < idx_macos,
        "RuboCop::Cop::Cask::StanzaOrder requires `{dep_formula}` before `{dep_macos}`"
    );

    for expected in [
        &format!("cask \"{}\" do", product::HOMEBREW_CASK),
        &format!("version \"{}\"", product::VERSION),
        &format!(
            "url \"https://github.com/quiinthieu/niwa/releases/download/v#{{version}}/{}-v#{{version}}-macos.dmg\"",
            product::DISPLAY_NAME
        ),
        &format!("name \"{}\"", product::DISPLAY_NAME),
        "homepage \"https://github.com/quiinthieu/niwa\"",
        &format!("app \"{}.app\"", product::DISPLAY_NAME),
        &format!("~/Library/Application Support/{}", product::DISPLAY_NAME),
        &format!("~/Library/Preferences/{}.plist", product::BUNDLE_ID),
        "caveats <<~EOS",
        "--no-quarantine",
        "xattr -dr com.apple.quarantine",
    ] {
        assert!(
            cask.contains(expected),
            "Casks/{}.rb missing expected snippet: {expected}",
            product::HOMEBREW_CASK
        );
    }
}

#[test]
fn release_and_ci_workflows_cover_all_t17_artifacts_and_signing_modes() {
    let root = workspace_root();
    let release_yml = fs::read_to_string(root.join(".github/workflows/release.yml")).unwrap();
    let dmg_script = fs::read_to_string(root.join("scripts/package-macos-dmg.sh")).unwrap();
    let ci_yml = fs::read_to_string(root.join(".github/workflows/ci.yml")).unwrap();
    let brew_ci_script = fs::read_to_string(root.join("scripts/verify-homebrew-ci.sh")).unwrap();

    for expected in [
        "v*",
        "workflow_dispatch:",
        "dry_run:",
        "aarch64-apple-darwin",
        "x86_64-apple-darwin",
        "lipo -create",
        "universal-apple-darwin.tar.gz",
        "x86_64-unknown-linux-gnu",
        "aarch64-unknown-linux-gnu",
        "SHA256SUMS.txt",
        "./scripts/package-macos-dmg.sh",
        "APPLE_CERTIFICATE_P12_BASE64",
    ] {
        assert!(
            release_yml.contains(expected),
            ".github/workflows/release.yml missing expected T17 requirement: {expected}"
        );
    }

    for expected in [
        "hdiutil create",
        "notarytool submit",
        "stapler staple",
        "spctl -a",
        "codesign --force --deep --sign -",
        "codesign --verify --deep --strict",
    ] {
        assert!(
            dmg_script.contains(expected) || release_yml.contains(expected),
            "DMG packaging + release workflow must cover {expected}"
        );
    }

    assert!(
        ci_yml.contains("./scripts/verify-homebrew-ci.sh"),
        ".github/workflows/ci.yml must run ./scripts/verify-homebrew-ci.sh on macOS"
    );
    for expected in ["brew audit --strict", "brew install --build-from-source"] {
        assert!(
            brew_ci_script.contains(expected),
            "scripts/verify-homebrew-ci.sh must run `{expected}`"
        );
    }
}
