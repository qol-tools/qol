use anyhow::{bail, Result};
use std::path::PathBuf;

pub(super) fn os_bucket() -> &'static str {
    qol_platform::Os::Macos.name()
}

pub(super) fn xdg_config_home() -> Result<PathBuf> {
    bail!("the XDG config home exists only on Linux")
}

pub(super) fn xdg_data_home() -> Result<PathBuf> {
    bail!("the XDG data home exists only on Linux")
}

pub(super) fn start_menu_programs_dir() -> Result<PathBuf> {
    bail!("the Start menu Programs folder exists only on Windows")
}

#[cfg(not(test))]
pub(in crate::paths) fn fallback_runtime_dir() -> Option<std::path::PathBuf> {
    Some(std::path::PathBuf::from(qol_conventions::RUNTIME_DIR_PATH))
}

#[cfg(test)]
pub(super) fn test_runtime_root() -> std::io::Result<tempfile::TempDir> {
    tempfile::tempdir_in("/tmp")
}
