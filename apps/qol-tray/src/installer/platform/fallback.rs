use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

fn unavailable<T>() -> Result<T> {
    bail!("qol-tray installation is unavailable on this platform")
}

pub(super) fn install_dir() -> Result<PathBuf> {
    unavailable()
}

pub(super) fn autostart_path() -> Result<PathBuf> {
    unavailable()
}

pub(super) fn start_now(_binary_path: &Path) -> Result<()> {
    unavailable()
}

pub(super) fn stop_running(_binary_path: &Path) -> Result<()> {
    unavailable()
}

pub(super) fn set_executable_permissions(_path: &Path) -> Result<()> {
    unavailable()
}

pub(super) fn prepare_atomic_replace(_installed_binary: &Path) -> Result<()> {
    unavailable()
}

pub(super) fn should_bootstrap_current_install(_binary_path: &Path) -> Result<bool> {
    Ok(false)
}

pub(super) fn remove_legacy_install() {}

pub(super) fn register_application(_binary_path: &Path) -> Result<()> {
    unavailable()
}

pub(super) fn warn_system_install_conflict() {}
