use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

pub fn install_dir() -> Result<PathBuf> {
    super::unix_common::install_dir()
}

pub fn autostart_path() -> Result<PathBuf> {
    let config_dir = dirs::config_dir().context("Could not determine config directory")?;
    Ok(config_dir.join("autostart").join("qol-tray.desktop"))
}

pub fn write_autostart_entry(binary_path: &Path) -> Result<()> {
    let path = autostart_path()?;
    let parent = path
        .parent()
        .context("Autostart path has no parent directory")?;
    fs::create_dir_all(parent)
        .with_context(|| format!("Failed to create directory {}", parent.display()))?;

    let desktop = format!(
        "[Desktop Entry]\nType=Application\nName=QoL Tray\nComment=Quality of Life Tray daemon for utility scripts\nExec={}\nIcon=applications-utilities\nTerminal=false\nCategories=Utility;\nStartupNotify=false\nX-GNOME-Autostart-enabled=true\n",
        binary_path.display()
    );

    fs::write(&path, desktop)
        .with_context(|| format!("Failed to write autostart file {}", path.display()))?;
    Ok(())
}

pub fn start_now(binary_path: &Path) -> Result<()> {
    if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
        println!("Skipping auto-start because no GUI session was detected.");
        return Ok(());
    }

    if super::unix_common::is_running("qol-tray") {
        return Ok(());
    }

    super::unix_common::start_now(binary_path)
}

pub fn stop_running(binary_path: &Path) -> Result<()> {
    super::unix_common::stop_running(binary_path, "qol-tray")
}

pub fn set_executable_permissions(path: &Path) -> Result<()> {
    super::unix_common::set_executable_permissions(path)
}

pub fn prepare_atomic_replace(_: &Path) -> Result<()> {
    Ok(())
}

pub fn copy_symlink(source: &Path, target: &Path) -> Result<()> {
    super::unix_common::copy_symlink(source, target)
}

pub fn on_file_copied(source: &Path, target: &Path) -> Result<()> {
    super::unix_common::on_file_copied(source, target)
}

pub fn bundled_binary_candidates(installer_path: &Path) -> Vec<PathBuf> {
    installer_path
        .parent()
        .map(|dir| vec![dir.join(super::binary_filename())])
        .unwrap_or_default()
}
