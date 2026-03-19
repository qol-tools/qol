use anyhow::{anyhow, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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

pub(super) fn register_application(binary_path: &Path) -> Result<()> {
    imp::register_application(binary_path)
}

pub(super) fn warn_system_install_conflict() {
    imp::warn_system_install_conflict()
}

pub(super) fn bundled_binary_candidates(installer_path: &Path) -> Vec<PathBuf> {
    installer_path
        .parent()
        .map(|dir| vec![dir.join(binary_filename())])
        .unwrap_or_default()
}

pub(super) fn write_text_file(path: &Path, content: &str) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("Autostart path has no parent directory"))?;
    fs::create_dir_all(parent)
        .with_context(|| format!("Failed to create directory {}", parent.display()))?;
    fs::write(path, content)
        .with_context(|| format!("Failed to write autostart file {}", path.display()))?;
    Ok(())
}

pub(super) fn spawn_detached(binary_path: &Path) -> Result<()> {
    Command::new(binary_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("Failed to start {}", binary_path.display()))?;
    Ok(())
}
