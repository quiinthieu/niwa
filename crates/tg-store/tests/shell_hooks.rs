#[path = "../../../tests/support/mod.rs"]
mod support;

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::Path;
use support::TestEnv;
use tg_core::product::CLI_BIN;
use tg_store::error::StoreError;
use tg_store::shell_hook::{
    backup_path_for_rc, install_shell_hook, render_hook_snippet, resolve_rc_path_from_env,
    uninstall_shell_hook, ShellKind, HOOK_BLOCK_END, HOOK_BLOCK_START,
};

fn env_lookup(vars: &[(&str, OsString)]) -> impl FnMut(&str) -> Option<OsString> {
    let map: HashMap<String, OsString> = vars
        .iter()
        .map(|(k, v)| ((*k).to_string(), v.clone()))
        .collect();
    move |key: &str| map.get(key).cloned()
}

#[test]
fn shell_kind_parses_and_detects_from_shell_env() {
    assert_eq!("zsh".parse::<ShellKind>(), Ok(ShellKind::Zsh));
    assert_eq!("bash".parse::<ShellKind>(), Ok(ShellKind::Bash));
    assert_eq!("fish".parse::<ShellKind>(), Ok(ShellKind::Fish));
    assert_eq!("ZSH".parse::<ShellKind>(), Ok(ShellKind::Zsh));
    assert!("pwsh".parse::<ShellKind>().is_err());

    assert_eq!(
        ShellKind::detect_from_shell_env(OsStr::new("/bin/zsh")),
        Some(ShellKind::Zsh)
    );
    assert_eq!(
        ShellKind::detect_from_shell_env(OsStr::new("/usr/local/bin/fish")),
        Some(ShellKind::Fish)
    );
    assert_eq!(
        ShellKind::detect_from_shell_env(OsStr::new("-bash")),
        Some(ShellKind::Bash)
    );
    assert_eq!(
        ShellKind::detect_from_shell_env(OsStr::new("/bin/csh")),
        None
    );
    assert_eq!(ShellKind::detect_from_shell_env(OsStr::new("")), None);
}

#[test]
fn rc_paths_resolve_isolated_locations_for_zsh_bash_and_fish() {
    let env = TestEnv::new();
    let home_os = env.home_dir().as_os_str().to_os_string();
    let xdg_os = env.xdg_config_home().as_os_str().to_os_string();
    let zdot_dir = env.home_dir().join("custom_zdot");
    let zdot_os = zdot_dir.as_os_str().to_os_string();

    let zsh_default = resolve_rc_path_from_env(
        ShellKind::Zsh,
        env_lookup(&[("HOME", home_os.clone())]),
        None,
    )
    .expect("resolve default .zshrc");
    assert_eq!(zsh_default, env.home_dir().join(".zshrc"));

    let zsh_zdotdir = resolve_rc_path_from_env(
        ShellKind::Zsh,
        env_lookup(&[("HOME", home_os.clone()), ("ZDOTDIR", zdot_os)]),
        None,
    )
    .expect("resolve ZDOTDIR/.zshrc");
    assert_eq!(zsh_zdotdir, zdot_dir.join(".zshrc"));

    let bash_rc = resolve_rc_path_from_env(
        ShellKind::Bash,
        env_lookup(&[("HOME", home_os.clone())]),
        None,
    )
    .expect("resolve .bashrc");
    assert_eq!(bash_rc, env.home_dir().join(".bashrc"));

    let fish_xdg = resolve_rc_path_from_env(
        ShellKind::Fish,
        env_lookup(&[("HOME", home_os.clone()), ("XDG_CONFIG_HOME", xdg_os)]),
        None,
    )
    .expect("resolve XDG fish config");
    assert_eq!(
        fish_xdg,
        env.xdg_config_home().join("fish").join("config.fish")
    );

    let fish_fallback =
        resolve_rc_path_from_env(ShellKind::Fish, env_lookup(&[("HOME", home_os)]), None)
            .expect("resolve fallback ~/.config/fish/config.fish");
    assert_eq!(
        fish_fallback,
        env.home_dir()
            .join(".config")
            .join("fish")
            .join("config.fish")
    );

    let missing = resolve_rc_path_from_env(ShellKind::Zsh, env_lookup(&[]), None);
    assert!(matches!(missing, Err(StoreError::MissingHomeDirectory)));
}

#[test]
fn rendered_snippets_background_ingest_preserve_privacy_and_quote_custom_binaries() {
    let custom_bin = Path::new("/opt/My Apps/niwa's bin/niwa");

    for shell in ShellKind::ALL {
        let default_snippet = render_hook_snippet(shell, None);
        assert!(
            default_snippet.contains(&format!("command {CLI_BIN} _ingest --exit")),
            "default snippet for {shell} must invoke product CLI_BIN _ingest:\n{default_snippet}"
        );
        assert!(
            default_snippet.contains("--dur"),
            "default snippet for {shell} must pass --dur:\n{default_snippet}"
        );
        assert!(
            default_snippet.contains("&"),
            "default snippet for {shell} must background _ingest:\n{default_snippet}"
        );

        // Hard privacy checks: snippets must never inspect positional command text, $PWD, or history.
        for forbidden in ["$1", "$2", "$3", "$argv", "$PWD", "pwd", "history", "fc "] {
            assert!(
                !default_snippet.contains(forbidden),
                "privacy violation: {shell} snippet referenced '{forbidden}':\n{default_snippet}"
            );
        }

        let custom_snippet = render_hook_snippet(shell, Some(custom_bin));
        assert!(
            custom_snippet.contains("_ingest --exit"),
            "custom-bin snippet for {shell} must invoke _ingest:\n{custom_snippet}"
        );
        match shell {
            ShellKind::Zsh | ShellKind::Bash => {
                assert!(
                    custom_snippet.contains("command '/opt/My Apps/niwa'\\''s bin/niwa' _ingest"),
                    "POSIX single-quote escaping expected for {shell}:\n{custom_snippet}"
                );
            }
            ShellKind::Fish => {
                assert!(
                    custom_snippet.contains("command '/opt/My Apps/niwa\\'s bin/niwa' _ingest"),
                    "Fish single-quote escaping expected for fish:\n{custom_snippet}"
                );
            }
        }
    }
}

#[test]
fn dry_run_install_without_write_rc_touches_zero_files() {
    let env = TestEnv::new();
    let rc_path = env.home_dir().join(".zshrc");
    let bak_path = backup_path_for_rc(&rc_path);

    let outcome =
        install_shell_hook(ShellKind::Zsh, &rc_path, None, false).expect("dry-run install");
    assert!(!outcome.wrote_rc);
    assert!(outcome.backup_path.is_none());
    assert!(outcome.rendered_block.contains(HOOK_BLOCK_START));
    assert!(outcome.rendered_block.contains(HOOK_BLOCK_END));
    assert!(
        !rc_path.exists(),
        "dry-run without --write-rc must not create rc file"
    );
    assert!(
        !bak_path.exists(),
        "dry-run without --write-rc must not create .niwa.bak"
    );

    let original = b"# existing user config\nexport EDITOR=vim\n";
    fs::write(&rc_path, original).expect("seed existing .zshrc");
    let outcome_existing = install_shell_hook(ShellKind::Zsh, &rc_path, None, false)
        .expect("dry-run install on existing rc");
    assert!(!outcome_existing.wrote_rc);
    assert_eq!(fs::read(&rc_path).expect("read .zshrc"), original);
    assert!(!bak_path.exists());
}

#[test]
fn install_with_write_rc_creates_backup_is_idempotent_and_uninstall_restores_exact_bytes() {
    let env = TestEnv::new();
    let rc_path = env.home_dir().join(".bashrc");
    let bak_path = backup_path_for_rc(&rc_path);

    let original = b"# user bashrc\nalias ll='ls -la'\nexport PATH=\"$HOME/bin:$PATH\"";
    fs::write(&rc_path, original).expect("seed .bashrc without trailing newline");

    let first =
        install_shell_hook(ShellKind::Bash, &rc_path, None, true).expect("first --write-rc");
    assert!(first.wrote_rc);
    assert_eq!(first.backup_path.as_deref(), Some(bak_path.as_path()));
    assert_eq!(
        fs::read(&bak_path).expect("backup holds exact pre-Niwa bytes"),
        original
    );

    let after_first = fs::read_to_string(&rc_path).expect("read hooked .bashrc");
    assert_eq!(after_first.matches(HOOK_BLOCK_START).count(), 1);
    assert_eq!(after_first.matches(HOOK_BLOCK_END).count(), 1);

    // Re-running `install --write-rc` (even with an updated `--bin` path) must update in-place
    // without duplicating blocks or overwriting `.niwa.bak` with hooked contents.
    let override_bin = Path::new("/usr/local/bin/niwa");
    let second = install_shell_hook(ShellKind::Bash, &rc_path, Some(override_bin), true)
        .expect("idempotent second --write-rc");
    assert!(second.wrote_rc);
    let after_second = fs::read_to_string(&rc_path).expect("read updated .bashrc");
    assert_eq!(after_second.matches(HOOK_BLOCK_START).count(), 1);
    assert_eq!(after_second.matches(HOOK_BLOCK_END).count(), 1);
    assert!(after_second.contains("command '/usr/local/bin/niwa' _ingest"));
    assert_eq!(
        fs::read(&bak_path).expect("pre-Niwa backup preserved across re-install"),
        original
    );

    let removed = uninstall_shell_hook(&rc_path).expect("uninstall hook");
    assert!(removed.removed);
    assert_eq!(
        fs::read(&rc_path).expect("restored .bashrc bytes"),
        original,
        "install -> install -> uninstall must restore rc file byte for byte"
    );
    assert!(
        !bak_path.exists(),
        "uninstall must clean up .niwa.bak after restoring rc"
    );

    // Second uninstall is a clean idempotent no-op.
    let second_uninstall = uninstall_shell_hook(&rc_path).expect("idempotent second uninstall");
    assert!(!second_uninstall.removed);
    assert_eq!(fs::read(&rc_path).expect("unchanged .bashrc"), original);
}

#[test]
fn install_on_nonexistent_rc_and_uninstall_removes_created_file_cleanly() {
    let env = TestEnv::new();
    let fish_rc = env.xdg_config_home().join("fish").join("config.fish");
    let bak_path = backup_path_for_rc(&fish_rc);
    assert!(!fish_rc.exists());

    let installed = install_shell_hook(ShellKind::Fish, &fish_rc, None, true)
        .expect("install into nonexistent config.fish");
    assert!(installed.wrote_rc);
    assert!(installed.backup_path.is_none());
    assert!(fish_rc.is_file());
    assert!(!bak_path.exists());

    let uninstalled = uninstall_shell_hook(&fish_rc).expect("uninstall created config.fish");
    assert!(uninstalled.removed);
    assert!(
        !fish_rc.exists(),
        "rc file created solely by Niwa must be removed on uninstall when empty"
    );
}

#[test]
fn uninstall_preserves_lines_added_after_marked_block() {
    let env = TestEnv::new();
    let rc_path = env.home_dir().join(".zshrc");
    fs::write(&rc_path, b"export FIRST=1").expect("seed .zshrc without trailing newline");

    install_shell_hook(ShellKind::Zsh, &rc_path, None, true).expect("install hook");
    let mut with_extra = fs::read(&rc_path).expect("read hooked .zshrc");
    with_extra.extend_from_slice(b"export ADDED_AFTER=2\n");
    fs::write(&rc_path, &with_extra).expect("append user edit after marked block");

    let removed = uninstall_shell_hook(&rc_path).expect("uninstall while keeping trailing edit");
    assert!(removed.removed);
    assert_eq!(
        fs::read(&rc_path).expect("read .zshrc after uninstall"),
        b"export FIRST=1\nexport ADDED_AFTER=2\n"
    );
}

#[test]
fn malformed_unclosed_marker_returns_error_without_modifying_rc() {
    let env = TestEnv::new();
    let rc_path = env.home_dir().join(".zshrc");
    let broken = format!("export A=1\n{HOOK_BLOCK_START}\n# missing end marker\n");
    fs::write(&rc_path, broken.as_bytes()).expect("write malformed .zshrc");

    let install_err = install_shell_hook(ShellKind::Zsh, &rc_path, None, true)
        .expect_err("unclosed marker must fail install");
    assert!(matches!(
        install_err,
        StoreError::MalformedShellHookBlock { .. }
    ));
    assert_eq!(fs::read(&rc_path).expect("rc untouched"), broken.as_bytes());

    let uninstall_err =
        uninstall_shell_hook(&rc_path).expect_err("unclosed marker must fail uninstall");
    assert!(matches!(
        uninstall_err,
        StoreError::MalformedShellHookBlock { .. }
    ));
    assert_eq!(fs::read(&rc_path).expect("rc untouched"), broken.as_bytes());
}

#[test]
fn install_then_uninstall_restores_boundary_rc_payloads_byte_for_byte() {
    let cases: &[(&str, &[u8])] = &[
        ("empty_zero_bytes", b""),
        ("single_lf", b"\n"),
        ("double_lf", b"\n\n"),
        ("triple_lf", b"\n\n\n"),
        ("single_crlf", b"\r\n"),
        ("double_crlf", b"\r\n\r\n"),
        ("no_trailing_newline", b"export FOO=bar"),
        ("trailing_lf", b"export FOO=bar\n"),
        ("trailing_crlf", b"export FOO=bar\r\n"),
        ("non_utf8_latin1_comment", b"# Caf\xe9 config\nexport A=1"),
        (
            "multiline_unicode_no_eof_nl",
            "# \u{5ead} Niwa user config\nalias g='git status'\nPATH=\"$HOME/.local/bin:$PATH\""
                .as_bytes(),
        ),
    ];

    let env = TestEnv::new();
    for (label, payload) in cases {
        for shell in ShellKind::ALL {
            for reinstall in [false, true] {
                let rc_path = env
                    .home_dir()
                    .join(format!("{label}_{shell}_{reinstall}.rc"));
                let bak_path = backup_path_for_rc(&rc_path);
                fs::write(&rc_path, payload).expect("seed boundary rc payload");

                install_shell_hook(shell, &rc_path, None, true).expect("install hook");
                assert_eq!(
                    &fs::read(&bak_path).expect("backup exists"),
                    payload,
                    "backup mismatch on {label} ({shell})"
                );

                if reinstall {
                    install_shell_hook(shell, &rc_path, Some(Path::new("/tmp/niwa")), true)
                        .expect("reinstall hook");
                    assert_eq!(
                        &fs::read(&bak_path).expect("backup preserved across reinstall"),
                        payload,
                        "reinstall clobbered backup on {label} ({shell})"
                    );
                }

                let removed = uninstall_shell_hook(&rc_path).expect("uninstall hook");
                assert!(removed.removed);
                assert!(!bak_path.exists());
                assert_eq!(
                    &fs::read(&rc_path).expect("read restored rc"),
                    payload,
                    "byte-for-byte mismatch after uninstall on {label} ({shell}, reinstall={reinstall})"
                );
            }
        }
    }
}
