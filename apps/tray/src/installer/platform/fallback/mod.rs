use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

use super::InstallerOps;

fn unavailable<T>() -> Result<T> {
    bail!("qol-tray installation is unavailable on this platform")
}

pub(super) struct Platform;

impl InstallerOps for Platform {
    fn binary_filename(&self) -> String {
        "qol-tray".to_string()
    }

    fn install_dir(&self) -> Result<PathBuf> {
        unavailable()
    }

    fn start_now(&self, _binary_path: &Path) -> Result<()> {
        unavailable()
    }

    fn stop_running(&self, _binary_path: &Path) -> Result<()> {
        unavailable()
    }

    fn set_executable_permissions(&self, _path: &Path) -> Result<()> {
        unavailable()
    }

    fn prepare_atomic_replace(&self, _installed_binary: &Path) -> Result<()> {
        unavailable()
    }

    fn should_bootstrap_current_install(&self, _binary_path: &Path) -> Result<bool> {
        Ok(false)
    }

    fn register_application(&self, _binary_path: &Path) -> Result<()> {
        unavailable()
    }

    fn warn_system_install_conflict(&self) {}

    fn remove_legacy_install(&self) {}
}
