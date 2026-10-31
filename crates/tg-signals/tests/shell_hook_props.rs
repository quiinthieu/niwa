#[path = "../../../tests/support/mod.rs"]
mod support;

use proptest::prelude::*;
use std::fs;
use std::path::Path;
use support::TestEnv;
use tg_store::shell_hook::{
    backup_path_for_rc, install_shell_hook, uninstall_shell_hook, ShellKind, HOOK_BLOCK_END,
    HOOK_BLOCK_START,
};

fn contains_hook_marker(bytes: &[u8]) -> bool {
    bytes
        .windows(HOOK_BLOCK_START.len())
        .any(|w| w == HOOK_BLOCK_START.as_bytes())
        || bytes
            .windows(HOOK_BLOCK_END.len())
            .any(|w| w == HOOK_BLOCK_END.as_bytes())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    #[test]
    fn prop_install_then_uninstall_restores_arbitrary_rc_byte_for_byte(
        original in prop::collection::vec(any::<u8>(), 0..512)
            .prop_filter("exclude pre-existing Niwa marker substrings", |b| !contains_hook_marker(b)),
        shell_idx in 0_usize..3,
        reinstall in any::<bool>(),
    ) {
        let env = TestEnv::new();
        let shell = ShellKind::ALL[shell_idx];
        let rc_path = env.home_dir().join("arbitrary.rc");
        let bak_path = backup_path_for_rc(&rc_path);

        fs::write(&rc_path, &original).expect("write arbitrary rc payload");

        install_shell_hook(shell, &rc_path, None, true).expect("install hook");
        prop_assert_eq!(&fs::read(&bak_path).expect("backup exists"), &original);

        if reinstall {
            install_shell_hook(shell, &rc_path, Some(Path::new("/tmp/niwa")), true)
                .expect("reinstall hook");
            prop_assert_eq!(&fs::read(&bak_path).expect("backup preserved"), &original);
        }

        let outcome = uninstall_shell_hook(&rc_path).expect("uninstall hook");
        prop_assert!(outcome.removed);
        prop_assert!(!bak_path.exists());

        let restored = fs::read(&rc_path).expect("restored rc exists");
        prop_assert_eq!(restored, original);
    }
}
