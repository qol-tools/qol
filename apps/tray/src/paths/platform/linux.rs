use anyhow::{bail, Context, Result};
use std::path::PathBuf;

pub(super) fn os_bucket() -> &'static str {
    qol_platform::Os::Linux.name()
}

pub(super) fn xdg_config_home() -> Result<PathBuf> {
    #[cfg(any(test, debug_assertions))]
    if let Some(home) = crate::paths::test_home_dir() {
        return Ok(home.join(".config"));
    }

    dirs::config_dir().context("Could not determine config directory")
}

pub(super) fn xdg_data_home() -> Result<PathBuf> {
    #[cfg(any(test, debug_assertions))]
    if let Some(home) = crate::paths::test_home_dir() {
        return Ok(home.join(".local").join("share"));
    }

    dirs::data_dir().context("Could not determine data directory")
}

pub(super) fn start_menu_programs_dir() -> Result<PathBuf> {
    bail!("the Start menu Programs folder exists only on Windows")
}

#[cfg(not(test))]
pub(in crate::paths) fn fallback_runtime_dir() -> Option<PathBuf> {
    Some(PathBuf::from(qol_conventions::RUNTIME_DIR_PATH))
}

#[cfg(test)]
pub(super) fn test_runtime_root() -> std::io::Result<tempfile::TempDir> {
    tempfile::tempdir()
}
