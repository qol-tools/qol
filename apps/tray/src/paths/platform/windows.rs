use anyhow::{bail, Context, Result};
use std::path::PathBuf;

pub(super) fn os_bucket() -> &'static str {
    qol_platform::Os::Windows.name()
}

pub(super) fn start_menu_programs_dir() -> Result<PathBuf> {
    #[cfg(any(test, debug_assertions))]
    if let Some(home) = crate::paths::test_home_dir() {
        return Ok(home.join(r"AppData\Roaming\Microsoft\Windows\Start Menu\Programs"));
    }

    qol_apps::known_folder::programs_dir()
        .context("Failed to resolve the Start menu Programs folder")
}

pub(super) fn xdg_config_home() -> Result<PathBuf> {
    bail!("the XDG config home exists only on Linux")
}

pub(super) fn xdg_data_home() -> Result<PathBuf> {
    bail!("the XDG data home exists only on Linux")
}

#[cfg(not(test))]
pub(in crate::paths) fn fallback_runtime_dir() -> Option<PathBuf> {
    None
}

#[cfg(test)]
pub(super) fn test_runtime_root() -> std::io::Result<tempfile::TempDir> {
    tempfile::tempdir()
}
