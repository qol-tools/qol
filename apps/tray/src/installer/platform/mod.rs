use anyhow::Result;
use qol_apps::tray_install::InstallProfile;
use std::path::{Path, PathBuf};

trait InstallerOps {
    fn binary_filename(&self) -> String;
    fn install_dir(&self, profile: InstallProfile) -> Result<PathBuf>;
    fn start_now(&self, binary_path: &Path) -> Result<()>;
    fn stop_running(&self, binary_path: &Path) -> Result<()>;
    fn set_executable_permissions(&self, path: &Path) -> Result<()>;
    fn should_bootstrap_current_install(&self, binary_path: &Path) -> Result<bool>;
    fn register_application(&self, binary_path: &Path) -> Result<()>;
    fn ensure_desktop_registration(&self) -> Result<()>;
    fn warn_system_install_conflict(&self);
    fn remove_legacy_install(&self);
    fn uninstall(&self, binary_path: &Path, profile: InstallProfile) -> Result<()>;
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
pub(crate) mod linux;
#[cfg(target_os = "macos")]
pub(crate) mod macos;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod unix_common;
#[cfg(target_os = "windows")]
pub(crate) mod windows;
#[cfg(any(target_os = "windows", test))]
pub(in crate::installer) mod windows_rules;

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

pub(crate) fn install_dir(profile: InstallProfile) -> Result<PathBuf> {
    Platform.install_dir(profile)
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

pub(super) fn should_bootstrap_current_install(binary_path: &Path) -> Result<bool> {
    Platform.should_bootstrap_current_install(binary_path)
}

pub(super) fn register_application(binary_path: &Path) -> Result<()> {
    Platform.register_application(binary_path)
}

pub(super) fn ensure_desktop_registration() -> Result<()> {
    Platform.ensure_desktop_registration()
}

pub(super) fn warn_system_install_conflict() {
    Platform.warn_system_install_conflict()
}

pub(super) fn remove_legacy_install() {
    Platform.remove_legacy_install();
}

pub(super) fn uninstall(binary_path: &Path, profile: InstallProfile) -> Result<()> {
    Platform.uninstall(binary_path, profile)
}

pub(super) fn tray_running() -> bool {
    let name = binary_filename();
    let own_pid = std::process::id();
    qol_process::processes().is_ok_and(|processes| {
        processes
            .iter()
            .any(|entry| entry.pid != own_pid && same_process_name(&entry.exe, &name))
    })
}

fn same_process_name(a: &str, b: &str) -> bool {
    a == b || (cfg!(windows) && a.eq_ignore_ascii_case(b))
}

pub(super) fn bundled_binary_candidates(installer_path: &Path) -> Vec<PathBuf> {
    installer_path
        .parent()
        .map(|dir| vec![dir.join(binary_filename())])
        .unwrap_or_default()
}
