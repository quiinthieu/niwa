use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/niwa sits two levels below workspace root")
        .to_path_buf()
}

fn lint_script() -> PathBuf {
    repo_root().join("scripts").join("lint-privacy.sh")
}

#[test]
fn clean_repo_passes_privacy_lint() {
    let output = Command::new("bash")
        .arg(lint_script())
        .output()
        .expect("lint-privacy.sh must execute");

    assert!(
        output.status.success(),
        "clean repo failed privacy lint:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn catches_planted_std_net_import() {
    let temp = TempDir::new().expect("temp workspace");
    let src_dir = temp.path().join("crates").join("tg-core").join("src");
    fs::create_dir_all(&src_dir).expect("create temp src dir");

    let planted = ["use std", "::", "net", "::", "Tcp", "Stream;\n"].concat();
    fs::write(src_dir.join("lib.rs"), planted).expect("write planted source");

    let output = Command::new("bash")
        .arg(lint_script())
        .arg(temp.path())
        .output()
        .expect("lint-privacy.sh must execute");

    assert!(
        !output.status.success(),
        "privacy lint should fail on planted std network import"
    );

    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let expected_token = ["std", "::", "net"].concat();
    assert!(
        combined.contains("Privacy violation") && combined.contains(&expected_token),
        "expected lint output to report Privacy violation with planted import, got:\n{combined}"
    );
}

#[test]
fn catches_grouped_std_net_import() {
    let temp = TempDir::new().expect("temp workspace");
    let src_dir = temp.path().join("crates").join("tg-core").join("src");
    fs::create_dir_all(&src_dir).expect("create temp src dir");

    let planted = ["use std::{io, ", "net", "::", "Tcp", "Listener};\n"].concat();
    fs::write(src_dir.join("lib.rs"), planted).expect("write planted grouped import");

    let output = Command::new("bash")
        .arg(lint_script())
        .arg(temp.path())
        .output()
        .expect("lint-privacy.sh must execute");

    assert!(
        !output.status.success(),
        "privacy lint should fail on grouped network listener import"
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("Privacy violation"),
        "expected Privacy violation message, got:\n{combined}"
    );
}

#[test]
fn catches_planted_network_crate_and_swift_api() {
    // 1. Forbidden crate in Cargo.toml
    let temp_cargo = TempDir::new().expect("temp workspace");
    let crates_dir = temp_cargo.path().join("crates").join("niwa");
    fs::create_dir_all(&crates_dir).expect("create temp crate dir");
    let cargo_toml = format!(
        "[package]\nname = \"sample\"\nversion = \"0.1.0\"\n\n[dependencies]\n{}\n",
        ["req", "west = \"0.12\""].concat()
    );
    fs::write(crates_dir.join("Cargo.toml"), cargo_toml).expect("write planted Cargo.toml");

    let cargo_out = Command::new("bash")
        .arg(lint_script())
        .arg(temp_cargo.path())
        .output()
        .expect("lint-privacy.sh must execute");
    assert!(
        !cargo_out.status.success(),
        "privacy lint should fail on forbidden networking crate in Cargo.toml"
    );
    let cargo_combined = format!(
        "{}{}",
        String::from_utf8_lossy(&cargo_out.stdout),
        String::from_utf8_lossy(&cargo_out.stderr)
    );
    assert!(
        cargo_combined.contains("Privacy violation"),
        "expected Privacy violation message for Cargo.toml, got:\n{cargo_combined}"
    );

    // 2. Forbidden HTTP session API in Swift source
    let temp_swift = TempDir::new().expect("temp workspace");
    let macos_dir = temp_swift.path().join("macos").join("Sources");
    fs::create_dir_all(&macos_dir).expect("create temp macos dir");
    let swift_src = format!("let session = {}.shared\n", ["URL", "Session"].concat());
    fs::write(macos_dir.join("App.swift"), swift_src).expect("write planted Swift file");

    let swift_out = Command::new("bash")
        .arg(lint_script())
        .arg(temp_swift.path())
        .output()
        .expect("lint-privacy.sh must execute");
    assert!(
        !swift_out.status.success(),
        "privacy lint should fail on forbidden HTTP session API usage in Swift"
    );
    let swift_combined = format!(
        "{}{}",
        String::from_utf8_lossy(&swift_out.stdout),
        String::from_utf8_lossy(&swift_out.stderr)
    );
    assert!(
        swift_combined.contains("Privacy violation"),
        "expected Privacy violation message for Swift source, got:\n{swift_combined}"
    );
}
