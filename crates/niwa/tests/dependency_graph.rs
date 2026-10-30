use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Command;
use tg_core::product;

fn workspace_manifest_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/niwa must sit two levels under workspace root")
        .join("Cargo.toml")
}

#[test]
fn dependency_graph_matches_plan_spec() {
    let output = Command::new(env!("CARGO"))
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--offline",
        ])
        .arg("--manifest-path")
        .arg(workspace_manifest_path())
        .output()
        .expect("cargo metadata must execute");

    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let metadata: Value =
        serde_json::from_slice(&output.stdout).expect("cargo metadata output must be valid JSON");

    let packages = metadata["packages"]
        .as_array()
        .expect("packages must be an array");

    let workspace_members: HashSet<String> = packages
        .iter()
        .map(|pkg| {
            pkg["name"]
                .as_str()
                .expect("package name must be string")
                .to_string()
        })
        .collect();

    let expected_members: HashSet<String> = [
        "tg-core",
        "tg-signals",
        "tg-store",
        "tg-render",
        product::CLI_CRATE,
    ]
    .into_iter()
    .map(str::to_string)
    .collect();

    assert_eq!(
        workspace_members, expected_members,
        "workspace members must match PLAN §3.1"
    );

    // Allowed third-party dependencies from PLAN §3.3.
    let allowed_runtime: HashSet<&str> = [
        "serde",
        "serde_json",
        "toml",
        "clap",
        "ratatui",
        "crossterm",
        "directories",
        "rand",
        "rand_pcg",
        "thiserror",
        "anyhow",
        "fs4",
        "fd-lock",
        "time",
        "jiff",
    ]
    .into_iter()
    .collect();

    let allowed_dev_only: HashSet<&str> = ["insta", "proptest", "assert_cmd", "tempfile"]
        .into_iter()
        .collect();

    let mut internal_deps: HashMap<String, HashSet<String>> = HashMap::new();

    for pkg in packages {
        let pkg_name = pkg["name"].as_str().expect("package name");
        let deps = pkg["dependencies"]
            .as_array()
            .expect("dependencies must be array");

        if pkg_name == product::CLI_CRATE {
            let bin_names: HashSet<&str> = pkg["targets"]
                .as_array()
                .expect("targets array")
                .iter()
                .filter(|target| {
                    target["kind"]
                        .as_array()
                        .is_some_and(|kinds| kinds.iter().any(|k| k.as_str() == Some("bin")))
                })
                .filter_map(|target| target["name"].as_str())
                .collect();
            assert!(
                bin_names.contains(product::CLI_BIN),
                "CLI crate '{}' must expose binary '{}', got {:?}",
                product::CLI_CRATE,
                product::CLI_BIN,
                bin_names
            );
        }

        let mut pkg_internal = HashSet::new();
        for dep in deps {
            let dep_name = dep["name"].as_str().expect("dep name");
            let dep_kind = dep["kind"].as_str(); // None => normal, Some("dev") => dev, Some("build") => build

            if workspace_members.contains(dep_name) {
                pkg_internal.insert(dep_name.to_string());
            } else {
                if dep_name == "anyhow" {
                    assert_eq!(
                        pkg_name,
                        product::CLI_CRATE,
                        "anyhow is allowed only in the binary crate (PLAN §3.3)"
                    );
                }

                match dep_kind {
                    Some("dev") => {
                        assert!(
                            allowed_runtime.contains(dep_name)
                                || allowed_dev_only.contains(dep_name),
                            "dev dependency '{dep_name}' in '{pkg_name}' is not in PLAN §3.3 (requires an ADR)"
                        );
                    }
                    _ => {
                        assert!(
                            allowed_runtime.contains(dep_name),
                            "dependency '{dep_name}' in '{pkg_name}' is not in PLAN §3.3 runtime allowlist (requires an ADR)"
                        );
                    }
                }
            }
        }
        internal_deps.insert(pkg_name.to_string(), pkg_internal);
    }

    // Expected internal dependencies based on PLAN §3.2:
    // tg-core -> {}
    // tg-store -> {tg-core}
    // tg-render -> {tg-core}
    // tg-signals -> {tg-core, tg-store}
    // niwa-garden -> {tg-core, tg-signals, tg-store, tg-render}
    assert_eq!(
        internal_deps.get("tg-core").unwrap(),
        &HashSet::new(),
        "tg-core must have no internal dependencies"
    );

    let only_core: HashSet<String> = ["tg-core".to_string()].into_iter().collect();
    assert_eq!(
        internal_deps.get("tg-store").unwrap(),
        &only_core,
        "tg-store must only depend on tg-core"
    );
    assert_eq!(
        internal_deps.get("tg-render").unwrap(),
        &only_core,
        "tg-render must only depend on tg-core"
    );

    let signals_expected: HashSet<String> = ["tg-core".to_string(), "tg-store".to_string()]
        .into_iter()
        .collect();
    assert_eq!(
        internal_deps.get("tg-signals").unwrap(),
        &signals_expected,
        "tg-signals must only depend on tg-core and tg-store"
    );

    let niwa_expected: HashSet<String> = [
        "tg-core".to_string(),
        "tg-signals".to_string(),
        "tg-store".to_string(),
        "tg-render".to_string(),
    ]
    .into_iter()
    .collect();
    assert_eq!(
        internal_deps.get(product::CLI_CRATE).unwrap(),
        &niwa_expected,
        "niwa-garden must depend on tg-core, tg-signals, tg-store, tg-render"
    );
}
