use anyhow::Result;
use std::path::PathBuf;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
use fallback as active;
#[cfg(target_os = "linux")]
use linux as active;
#[cfg(target_os = "macos")]
use macos as active;
#[cfg(target_os = "windows")]
use windows as active;

#[cfg(not(test))]
pub(super) use active::fallback_runtime_dir;

pub(super) fn xdg_config_home() -> Result<PathBuf> {
    active::xdg_config_home()
}

pub(super) fn xdg_data_home() -> Result<PathBuf> {
    active::xdg_data_home()
}

pub(super) fn start_menu_programs_dir() -> Result<PathBuf> {
    active::start_menu_programs_dir()
}

pub(super) fn os_bucket() -> &'static str {
    active::os_bucket()
}

#[cfg(test)]
pub(super) fn test_runtime_root() -> std::io::Result<tempfile::TempDir> {
    active::test_runtime_root()
}
