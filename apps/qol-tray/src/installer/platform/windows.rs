use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub fn install_dir() -> Result<PathBuf> {
    let local_data = dirs::data_local_dir().context("Could not determine local data directory")?;
    Ok(local_data.join("Programs").join("qol-tray").join("bin"))
}

pub fn autostart_path() -> Result<PathBuf> {
    let app_data = std::env::var_os("APPDATA").context("APPDATA is not set")?;
    Ok(PathBuf::from(app_data)
        .join("Microsoft")
        .join("Windows")
        .join("Start Menu")
        .join("Programs")
        .join("Startup")
        .join("qol-tray.cmd"))
}

pub fn write_autostart_entry(binary_path: &Path) -> Result<()> {
    let path = autostart_path()?;
    let parent = path
        .parent()
        .context("Autostart path has no parent directory")?;
    fs::create_dir_all(parent)
        .with_context(|| format!("Failed to create directory {}", parent.display()))?;

    let binary = binary_path.display().to_string().replace('\"', "\"\"");
    let content = format!("@echo off\r\nstart \"\" \"{}\"\r\n", binary);

    fs::write(&path, content)
        .with_context(|| format!("Failed to write autostart file {}", path.display()))?;
    Ok(())
}

pub fn start_now(binary_path: &Path) -> Result<()> {
    Command::new(binary_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("Failed to start {}", binary_path.display()))?;
    Ok(())
}

pub fn stop_running(_: &Path) -> Result<()> {
    let _ = Command::new("taskkill")
        .args(["/F", "/IM", "qol-tray.exe"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    Ok(())
}

pub fn set_executable_permissions(_: &Path) -> Result<()> {
    Ok(())
}

pub fn prepare_atomic_replace(installed_binary: &Path) -> Result<()> {
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

pub fn copy_symlink(source: &Path, target: &Path) -> Result<()> {
    let link_target = fs::read_link(source)
        .with_context(|| format!("Failed to read symlink {}", source.display()))?;
    let resolved = source
        .canonicalize()
        .with_context(|| format!("Failed to resolve symlink {}", source.display()))?;
    if resolved.is_dir() {
        std::os::windows::fs::symlink_dir(&link_target, target)
            .with_context(|| format!("Failed to create directory symlink {}", target.display()))?;
    } else {
        std::os::windows::fs::symlink_file(&link_target, target)
            .with_context(|| format!("Failed to create file symlink {}", target.display()))?;
    }
    Ok(())
}

pub fn on_file_copied(_: &Path, _: &Path) -> Result<()> {
    Ok(())
}

pub fn bundled_binary_candidates(installer_path: &Path) -> Vec<PathBuf> {
    installer_path
        .parent()
        .map(|dir| vec![dir.join(super::binary_filename())])
        .unwrap_or_default()
}
