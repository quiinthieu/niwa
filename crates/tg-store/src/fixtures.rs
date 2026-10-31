//! Loaders and stagers for `tests/fixtures/{saves,spools}/` test data.

use crate::paths::NiwaPaths;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

/// Accessor for a `tests/fixtures/` directory tree containing `saves/` and `spools/`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FixtureDir {
    root: PathBuf,
}

impl FixtureDir {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[must_use]
    pub fn saves_dir(&self) -> PathBuf {
        self.root.join("saves")
    }

    #[must_use]
    pub fn spools_dir(&self) -> PathBuf {
        self.root.join("spools")
    }

    pub fn read_save(&self, name: &str) -> io::Result<String> {
        let path = resolve_fixture_file(&self.saves_dir(), name)?;
        fs::read_to_string(path)
    }

    pub fn read_spool(&self, name: &str) -> io::Result<String> {
        let path = resolve_fixture_file(&self.spools_dir(), name)?;
        fs::read_to_string(path)
    }

    /// Copies `saves/<name>` into `paths.garden_file()`.
    pub fn stage_save(&self, name: &str, paths: &NiwaPaths) -> io::Result<PathBuf> {
        let src = resolve_fixture_file(&self.saves_dir(), name)?;
        paths.ensure_dirs()?;
        let dst = paths.garden_file();
        fs::copy(src, &dst)?;
        Ok(dst)
    }

    /// Copies `spools/<name>` into `paths.events_file()`.
    pub fn stage_spool(&self, name: &str, paths: &NiwaPaths) -> io::Result<PathBuf> {
        let src = resolve_fixture_file(&self.spools_dir(), name)?;
        paths.ensure_dirs()?;
        let dst = paths.events_file();
        fs::copy(src, &dst)?;
        Ok(dst)
    }
}

fn resolve_fixture_file(dir: &Path, name: &str) -> io::Result<PathBuf> {
    let mut components = Path::new(name).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(_)), None) => Ok(dir.join(name)),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("fixture name must be a single file name, got '{name}'"),
        )),
    }
}
