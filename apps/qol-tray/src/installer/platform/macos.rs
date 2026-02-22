use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

pub fn install_dir() -> Result<PathBuf> {
    super::unix_common::install_dir()
}

pub fn autostart_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Could not determine home directory")?;
    Ok(home
        .join("Library")
        .join("LaunchAgents")
        .join("com.qol-tools.qol-tray.plist"))
}

pub fn write_autostart_entry(binary_path: &Path) -> Result<()> {
    let path = autostart_path()?;
    let parent = path
        .parent()
        .context("Autostart path has no parent directory")?;
    fs::create_dir_all(parent)
        .with_context(|| format!("Failed to create directory {}", parent.display()))?;

    let binary = xml_escape(&binary_path.display().to_string());
    let plist = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n<key>Label</key>\n<string>com.qol-tools.qol-tray</string>\n<key>ProgramArguments</key>\n<array>\n<string>{}</string>\n</array>\n<key>RunAtLoad</key>\n<true/>\n<key>KeepAlive</key>\n<false/>\n</dict>\n</plist>\n",
        binary
    );

    fs::write(&path, plist)
        .with_context(|| format!("Failed to write autostart file {}", path.display()))?;
    Ok(())
}

pub fn start_now(binary_path: &Path) -> Result<()> {
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

fn xml_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
