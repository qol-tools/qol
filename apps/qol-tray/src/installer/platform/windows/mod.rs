use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::InstallerOps;

pub(super) struct Platform;

impl InstallerOps for Platform {
    fn binary_filename(&self) -> String {
        "qol-tray.exe".to_string()
    }

    fn install_dir(&self) -> Result<PathBuf> {
        let local_data =
            dirs::data_local_dir().context("Could not determine local data directory")?;
        Ok(local_data.join("Programs").join("qol-tray").join("bin"))
    }

    fn start_now(&self, binary_path: &Path) -> Result<()> {
        super::spawn_detached(binary_path)
    }

    fn stop_running(&self, _binary_path: &Path) -> Result<()> {
        let _ = Command::new("taskkill")
            .args(["/F", "/IM", "qol-tray.exe"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        Ok(())
    }

    fn set_executable_permissions(&self, _path: &Path) -> Result<()> {
        Ok(())
    }

    fn prepare_atomic_replace(&self, installed_binary: &Path) -> Result<()> {
        if !installed_binary.exists() {
            return Ok(());
        }
        fs::remove_file(installed_binary).with_context(|| {
            format!(
                "Failed to remove existing installed binary {}",
                installed_binary.display()
            )
        })
    }

    fn should_bootstrap_current_install(&self, _binary_path: &Path) -> Result<bool> {
        Ok(false)
    }

    fn register_application(&self, _binary_path: &Path) -> Result<()> {
        Ok(())
    }

    fn warn_system_install_conflict(&self) {}

    fn remove_legacy_install(&self) {}
}
