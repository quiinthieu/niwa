//! Shared `TestEnv` test harness isolating `NIWA_HOME`, `HOME`, and XDG paths
//! inside a temporary directory and providing workspace fixture loaders.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use tempfile::TempDir;
use tg_store::fixtures::FixtureDir;
use tg_store::paths::NiwaPaths;

pub struct TestEnv {
    _temp_dir: TempDir,
    home_dir: PathBuf,
    niwa_home: PathBuf,
    xdg_config_home: PathBuf,
    xdg_state_home: PathBuf,
    paths: NiwaPaths,
    fixtures: FixtureDir,
}

impl TestEnv {
    #[must_use]
    pub fn new() -> Self {
        let temp_dir = TempDir::new().expect("create temporary TestEnv root");
        let base = temp_dir.path();

        let home_dir = base.join("home");
        let niwa_home = base.join("niwa_home");
        let xdg_config_home = base.join("xdg_config");
        let xdg_state_home = base.join("xdg_state");

        // Delegate directory creation to `tg-store` (the sole disk writer in production/tests).
        NiwaPaths::new(&home_dir, &xdg_config_home)
            .ensure_dirs()
            .expect("create isolated HOME and XDG_CONFIG_HOME");
        NiwaPaths::from_niwa_home(&xdg_state_home)
            .ensure_dirs()
            .expect("create isolated XDG_STATE_HOME");

        let paths = NiwaPaths::from_niwa_home(&niwa_home);
        paths.ensure_dirs().expect("create isolated NIWA_HOME");

        let fixtures = FixtureDir::new(workspace_root().join("tests").join("fixtures"));

        Self {
            _temp_dir: temp_dir,
            home_dir,
            niwa_home,
            xdg_config_home,
            xdg_state_home,
            paths,
            fixtures,
        }
    }

    #[must_use]
    pub fn niwa_home(&self) -> &Path {
        &self.niwa_home
    }

    #[must_use]
    pub fn home_dir(&self) -> &Path {
        &self.home_dir
    }

    #[must_use]
    pub fn xdg_config_home(&self) -> &Path {
        &self.xdg_config_home
    }

    #[must_use]
    pub fn xdg_state_home(&self) -> &Path {
        &self.xdg_state_home
    }

    #[must_use]
    pub fn paths(&self) -> &NiwaPaths {
        &self.paths
    }

    #[must_use]
    pub fn fixtures(&self) -> &FixtureDir {
        &self.fixtures
    }

    /// Environment variable overrides isolating CLI processes from the real `$HOME`.
    #[must_use]
    pub fn env_vars(&self) -> [(&'static str, &Path); 4] {
        [
            ("NIWA_HOME", &self.niwa_home),
            ("HOME", &self.home_dir),
            ("XDG_CONFIG_HOME", &self.xdg_config_home),
            ("XDG_STATE_HOME", &self.xdg_state_home),
        ]
    }

    #[must_use]
    pub fn read_save_fixture(&self, name: &str) -> String {
        self.fixtures
            .read_save(name)
            .unwrap_or_else(|err| panic!("failed to read save fixture '{name}': {err}"))
    }

    #[must_use]
    pub fn read_spool_fixture(&self, name: &str) -> String {
        self.fixtures
            .read_spool(name)
            .unwrap_or_else(|err| panic!("failed to read spool fixture '{name}': {err}"))
    }

    #[must_use]
    pub fn stage_save_fixture(&self, name: &str) -> PathBuf {
        self.fixtures
            .stage_save(name, &self.paths)
            .unwrap_or_else(|err| panic!("failed to stage save fixture '{name}': {err}"))
    }

    #[must_use]
    pub fn stage_spool_fixture(&self, name: &str) -> PathBuf {
        self.fixtures
            .stage_spool(name, &self.paths)
            .unwrap_or_else(|err| panic!("failed to stage spool fixture '{name}': {err}"))
    }
}

impl Default for TestEnv {
    fn default() -> Self {
        Self::new()
    }
}

#[must_use]
pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crate manifest sits two levels below workspace root")
        .to_path_buf()
}
