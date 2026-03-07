use anyhow::Result;
use std::path::{Path, PathBuf};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod unix_common;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(target_os = "windows")]
use windows as imp;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
compile_error!("installer platform implementation is required for this target OS");

pub(super) fn binary_filename() -> String {
    if cfg!(target_os = "windows") {
        "qol-tray.exe".to_string()
    } else {
        "qol-tray".to_string()
    }
}

pub(super) fn install_dir() -> Result<PathBuf> {
    imp::install_dir()
}

pub(super) fn autostart_path() -> Result<PathBuf> {
    imp::autostart_path()
}

pub(super) fn write_autostart_entry(binary_path: &Path) -> Result<()> {
    imp::write_autostart_entry(binary_path)
}

pub(super) fn start_now(binary_path: &Path) -> Result<()> {
    imp::start_now(binary_path)
}

pub(super) fn stop_running(binary_path: &Path) -> Result<()> {
    imp::stop_running(binary_path)
}

pub(super) fn set_executable_permissions(path: &Path) -> Result<()> {
    imp::set_executable_permissions(path)
}

pub(super) fn prepare_atomic_replace(installed_binary: &Path) -> Result<()> {
    imp::prepare_atomic_replace(installed_binary)
}

pub(super) fn copy_symlink(source: &Path, target: &Path) -> Result<()> {
    imp::copy_symlink(source, target)
}

pub(super) fn on_file_copied(source: &Path, target: &Path) -> Result<()> {
    imp::on_file_copied(source, target)
}

pub(super) fn bundled_binary_candidates(installer_path: &Path) -> Vec<PathBuf> {
    imp::bundled_binary_candidates(installer_path)
}
