#[path = "../../../tests/support/mod.rs"]
mod support;

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use support::{workspace_root, TestEnv};
use tg_core::product::{BUNDLE_ID, CLI_BIN};
use tg_store::daemon_units::{
    install_daemon_unit, launchd_daemon_label, launchd_plist_filename, render_daemon_unit,
    render_launchd_plist, render_systemd_service, systemd_service_filename, uninstall_daemon_unit,
    DaemonUnitPaths,
};
use tg_store::error::StoreError;
use tg_store::paths::PlatformKind;

fn env_lookup(vars: &[(&str, OsString)]) -> impl FnMut(&str) -> Option<OsString> {
    let map: HashMap<String, OsString> = vars
        .iter()
        .map(|(k, v)| ((*k).to_string(), v.clone()))
        .collect();
    move |key: &str| map.get(key).cloned()
}

#[test]
fn daemon_unit_paths_resolve_launchd_and_systemd_locations_in_temp_home() {
    let env = TestEnv::new();

    let mac_paths = DaemonUnitPaths::resolve_from_env(
        PlatformKind::MacOs,
        env_lookup(&[("HOME", env.home_dir().as_os_str().to_os_string())]),
        None,
    )
    .expect("resolve macOS LaunchAgents path");
    assert_eq!(mac_paths.platform(), PlatformKind::MacOs);
    assert_eq!(
        mac_paths.unit_identifier(),
        format!("{BUNDLE_ID}.daemon").as_str()
    );
    assert_eq!(
        mac_paths.unit_path(),
        env.home_dir()
            .join("Library")
            .join("LaunchAgents")
            .join(format!("{BUNDLE_ID}.daemon.plist"))
    );

    let linux_xdg = DaemonUnitPaths::resolve_from_env(
        PlatformKind::Linux,
        env_lookup(&[
            ("HOME", env.home_dir().as_os_str().to_os_string()),
            (
                "XDG_CONFIG_HOME",
                env.xdg_config_home().as_os_str().to_os_string(),
            ),
        ]),
        None,
    )
    .expect("resolve Linux XDG_CONFIG_HOME systemd user unit path");
    assert_eq!(linux_xdg.platform(), PlatformKind::Linux);
    assert_eq!(
        linux_xdg.unit_identifier(),
        format!("{CLI_BIN}.service").as_str()
    );
    assert_eq!(
        linux_xdg.unit_path(),
        env.xdg_config_home()
            .join("systemd")
            .join("user")
            .join(format!("{CLI_BIN}.service"))
    );

    let linux_fallback = DaemonUnitPaths::resolve_from_env(
        PlatformKind::Linux,
        env_lookup(&[
            ("HOME", env.home_dir().as_os_str().to_os_string()),
            ("XDG_CONFIG_HOME", OsString::from("")),
        ]),
        None,
    )
    .expect("resolve Linux ~/.config/systemd/user fallback");
    assert_eq!(
        linux_fallback.unit_path(),
        env.home_dir()
            .join(".config")
            .join("systemd")
            .join("user")
            .join(format!("{CLI_BIN}.service"))
    );

    let missing_home =
        DaemonUnitPaths::resolve_from_env(PlatformKind::MacOs, env_lookup(&[]), None);
    assert!(matches!(
        missing_home,
        Err(StoreError::MissingHomeDirectory)
    ));
}

#[test]
fn rendered_units_match_packaging_templates_and_power_budget_directives() {
    let placeholder_bin = Path::new("@NIWA_BIN@");

    let launchd_template_path = workspace_root()
        .join("packaging")
        .join("launchd")
        .join(format!("{}.in", launchd_plist_filename()));
    let expected_plist =
        fs::read_to_string(&launchd_template_path).expect("read packaging launchd template");
    let rendered_plist = render_launchd_plist(placeholder_bin);
    assert_eq!(rendered_plist, expected_plist);
    assert_eq!(
        render_daemon_unit(PlatformKind::MacOs, placeholder_bin),
        expected_plist
    );
    assert!(rendered_plist.contains(&launchd_daemon_label()));
    assert!(rendered_plist.contains("<key>ProcessType</key>\n    <string>Background</string>"));
    assert!(rendered_plist.contains("<key>SuccessfulExit</key>\n        <false/>"));
    assert!(
        !rendered_plist.contains("StartInterval"),
        "launchd unit must not use StartInterval so exit 0 with no signals produces zero wakes"
    );

    let systemd_template_path = workspace_root()
        .join("packaging")
        .join("systemd")
        .join(format!("{}.in", systemd_service_filename()));
    let expected_service =
        fs::read_to_string(&systemd_template_path).expect("read packaging systemd template");
    let rendered_service = render_systemd_service(placeholder_bin);
    assert_eq!(rendered_service, expected_service);
    assert_eq!(
        render_daemon_unit(PlatformKind::Linux, placeholder_bin),
        expected_service
    );
    assert!(rendered_service.contains("Restart=on-failure"));
    assert!(
        !rendered_service.contains("Restart=always"),
        "systemd unit must not use Restart=always so exit 0 with no signals stays inactive"
    );
}

#[test]
fn rendered_units_escape_special_characters_and_spaces_in_binary_paths() {
    let tricky_mac_bin = Path::new("/Users/Alice & Bob/Apps/<Niwa> \"Dev\" '1'/niwa");
    let plist = render_launchd_plist(tricky_mac_bin);
    assert!(plist.contains(
        "<string>/Users/Alice &amp; Bob/Apps/&lt;Niwa&gt; &quot;Dev&quot; &apos;1&apos;/niwa</string>"
    ));

    let tricky_linux_bin = Path::new("/home/alice/My Apps/100% \"niwa\"\\bin/niwa");
    let service = render_systemd_service(tricky_linux_bin);
    assert!(
        service.contains("ExecStart=\"/home/alice/My Apps/100%% \\\"niwa\\\"\\\\bin/niwa\" daemon")
    );
}

#[test]
fn install_and_uninstall_round_trip_cleanly_in_test_env_for_both_platforms() {
    let env = TestEnv::new();
    let bin_v1 = PathBuf::from("/opt/homebrew/bin/niwa");
    let bin_v2 = PathBuf::from("/usr/local/bin/niwa");

    for platform in [PlatformKind::MacOs, PlatformKind::Linux] {
        let unit_paths = DaemonUnitPaths::resolve_from_env(
            platform,
            env_lookup(&[
                ("HOME", env.home_dir().as_os_str().to_os_string()),
                (
                    "XDG_CONFIG_HOME",
                    env.xdg_config_home().as_os_str().to_os_string(),
                ),
            ]),
            None,
        )
        .expect("resolve isolated temp-home daemon unit paths");

        assert!(!unit_paths.is_installed());
        assert!(!unit_paths.unit_path().exists());

        // Install v1.
        let installed_v1 =
            install_daemon_unit(&unit_paths, &bin_v1).expect("install daemon unit v1");
        assert_eq!(installed_v1.platform, platform);
        assert_eq!(installed_v1.unit_path, unit_paths.unit_path());
        assert!(unit_paths.is_installed());
        let disk_v1 = fs::read_to_string(unit_paths.unit_path()).expect("read installed unit v1");
        assert_eq!(disk_v1, render_daemon_unit(platform, &bin_v1));

        // Ensure atomic write left zero `.tmp.` sibling files behind.
        let parent_entries: Vec<String> = fs::read_dir(unit_paths.unit_path().parent().unwrap())
            .expect("list unit parent directory")
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(parent_entries.len(), 1);

        // Re-install v2 (atomic overwrite).
        let installed_v2 =
            install_daemon_unit(&unit_paths, &bin_v2).expect("re-install daemon unit v2");
        assert_eq!(installed_v2.contents, render_daemon_unit(platform, &bin_v2));
        let disk_v2 = fs::read_to_string(unit_paths.unit_path()).expect("read installed unit v2");
        assert_eq!(disk_v2, render_daemon_unit(platform, &bin_v2));

        // Uninstall removes the unit file cleanly.
        let uninstalled = uninstall_daemon_unit(&unit_paths).expect("uninstall daemon unit");
        assert!(uninstalled.removed);
        assert!(!unit_paths.is_installed());
        assert!(!unit_paths.unit_path().exists());

        // Second uninstall is idempotent.
        let uninstalled_again =
            uninstall_daemon_unit(&unit_paths).expect("idempotent second uninstall");
        assert!(!uninstalled_again.removed);
    }
}
