use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub fn install_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Could not determine home directory")?;
    Ok(home.join(".local").join("bin"))
}

pub fn autostart_path() -> Result<PathBuf> {
    let config_dir = dirs::config_dir().context("Could not determine config directory")?;
    Ok(config_dir.join("autostart").join("qol-tray.desktop"))
}

pub fn write_autostart_entry(binary_path: &Path) -> Result<()> {
    let path = autostart_path()?;
    let parent = path.parent().context("Autostart path has no parent directory")?;
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

    let is_running = Command::new("pgrep")
        .arg("-x")
        .arg("qol-tray")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false);

    if is_running {
        return Ok(());
    }

    Command::new(binary_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("Failed to start {}", binary_path.display()))?;

    Ok(())
}
