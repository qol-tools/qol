use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

trait InstallerOps {
    fn binary_filename(&self) -> String;
    fn install_dir(&self) -> Result<PathBuf>;
    fn start_now(&self, binary_path: &Path) -> Result<()>;
    fn stop_running(&self, binary_path: &Path) -> Result<()>;
    fn set_executable_permissions(&self, path: &Path) -> Result<()>;
    fn prepare_atomic_replace(&self, installed_binary: &Path) -> Result<()>;
    fn should_bootstrap_current_install(&self, binary_path: &Path) -> Result<bool>;
    fn register_application(&self, binary_path: &Path) -> Result<()>;
    fn warn_system_install_conflict(&self);
    fn remove_legacy_install(&self);
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
pub(super) mod linux;
#[cfg(target_os = "macos")]
pub(super) mod macos;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod unix_common;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
use fallback::Platform;
#[cfg(target_os = "linux")]
use linux::Platform;
#[cfg(target_os = "macos")]
use macos::Platform;
#[cfg(target_os = "windows")]
use windows::Platform;

pub(crate) fn binary_filename() -> String {
    Platform.binary_filename()
}

pub(crate) fn install_dir() -> Result<PathBuf> {
    Platform.install_dir()
}

pub(super) fn start_now(binary_path: &Path) -> Result<()> {
    Platform.start_now(binary_path)
}

pub(super) fn stop_running(binary_path: &Path) -> Result<()> {
    Platform.stop_running(binary_path)
}

pub(super) fn set_executable_permissions(path: &Path) -> Result<()> {
    Platform.set_executable_permissions(path)
}

pub(super) fn prepare_atomic_replace(installed_binary: &Path) -> Result<()> {
    Platform.prepare_atomic_replace(installed_binary)
}

pub(super) fn should_bootstrap_current_install(binary_path: &Path) -> Result<bool> {
    Platform.should_bootstrap_current_install(binary_path)
}

pub(super) fn register_application(binary_path: &Path) -> Result<()> {
    Platform.register_application(binary_path)
}

#[cfg(target_os = "linux")]
pub(crate) fn ensure_desktop_entries(binary_path: &Path) -> Result<()> {
    linux::ensure_linux_desktop_entries(binary_path)
}

pub(super) fn warn_system_install_conflict() {
    Platform.warn_system_install_conflict()
}

pub(super) fn remove_legacy_install() {
    Platform.remove_legacy_install();
}

pub(super) fn bundled_binary_candidates(installer_path: &Path) -> Vec<PathBuf> {
    installer_path
        .parent()
        .map(|dir| vec![dir.join(binary_filename())])
        .unwrap_or_default()
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
