use assert_cmd::Command;
use tempfile::TempDir;
use tg_core::product;

#[test]
fn version_flag_uses_product_constants() {
    let temp_home = TempDir::new().expect("temp NIWA_HOME");
    let expected = format!("{} {}\n", product::CLI_BIN, product::VERSION);

    for flag in ["--version", "-V"] {
        let assert = Command::new(env!("CARGO_BIN_EXE_niwa"))
            .env("NIWA_HOME", temp_home.path())
            .env("HOME", temp_home.path())
            .arg(flag)
            .assert()
            .success();

        let stdout =
            String::from_utf8(assert.get_output().stdout.clone()).expect("UTF-8 version output");
        assert_eq!(
            stdout, expected,
            "`niwa {flag}` output must match product::CLI_BIN and product::VERSION"
        );
    }
}
