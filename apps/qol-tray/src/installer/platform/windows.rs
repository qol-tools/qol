use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub(super) fn install_dir() -> Result<PathBuf> {
    let local_data = dirs::data_local_dir().context("Could not determine local data directory")?;
    Ok(local_data.join("Programs").join("qol-tray").join("bin"))
}

pub(super) fn autostart_path() -> Result<PathBuf> {
    let app_data = std::env::var_os("APPDATA").context("APPDATA is not set")?;
    Ok(PathBuf::from(app_data)
        .join("Microsoft")
        .join("Windows")
        .join("Start Menu")
        .join("Programs")
        .join("Startup")
        .join("qol-tray.cmd"))
}

pub(super) fn write_autostart_entry(binary_path: &Path) -> Result<()> {
    let path = autostart_path()?;
    let binary = binary_path.display().to_string().replace('\"', "\"\"");
    let content = format!("@echo off\r\nstart \"\" \"{}\"\r\n", binary);
    super::write_text_file(&path, &content)
}

pub(super) fn start_now(binary_path: &Path) -> Result<()> {
    super::spawn_detached(binary_path)
}

pub(super) fn stop_running(_: &Path) -> Result<()> {
    let _ = Command::new("taskkill")
        .args(["/F", "/IM", "qol-tray.exe"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    Ok(())
}

pub(super) fn set_executable_permissions(_: &Path) -> Result<()> {
    Ok(())
}

pub(super) fn prepare_atomic_replace(installed_binary: &Path) -> Result<()> {
    if installed_binary.exists() {
        fs::remove_file(installed_binary).with_context(|| {
            format!(
                "Failed to remove existing installed binary {}",
                installed_binary.display()
            )
        })?;
    }
    Ok(())
}

pub(super) fn register_application(_binary_path: &Path) -> Result<()> {
    Ok(())
}

pub(super) fn warn_system_install_conflict() {}
