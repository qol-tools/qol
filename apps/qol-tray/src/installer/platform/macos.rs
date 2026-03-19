use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub(super) fn install_dir() -> Result<PathBuf> {
    super::unix_common::install_dir()
}

pub(super) fn autostart_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Could not determine home directory")?;
    Ok(home
        .join("Library")
        .join("LaunchAgents")
        .join("com.qol-tools.qol-tray.plist"))
}

pub(super) fn write_autostart_entry(binary_path: &Path) -> Result<()> {
    let path = autostart_path()?;
    let binary = xml_escape(&binary_path.display().to_string());
    let plist = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n<key>Label</key>\n<string>com.qol-tools.qol-tray</string>\n<key>ProgramArguments</key>\n<array>\n<string>{}</string>\n</array>\n<key>RunAtLoad</key>\n<true/>\n<key>KeepAlive</key>\n<false/>\n</dict>\n</plist>\n",
        binary
    );
    super::write_text_file(&path, &plist)
}

pub(super) fn start_now(binary_path: &Path) -> Result<()> {
    if super::unix_common::is_running("qol-tray") {
        return Ok(());
    }

    super::unix_common::start_now(binary_path)
}

pub(super) fn stop_running(binary_path: &Path) -> Result<()> {
    super::unix_common::stop_running(binary_path, "qol-tray")
}

pub(super) fn set_executable_permissions(path: &Path) -> Result<()> {
    super::unix_common::set_executable_permissions(path)
}

pub(super) fn prepare_atomic_replace(_: &Path) -> Result<()> {
    Ok(())
}

pub(super) fn register_application(_binary_path: &Path) -> Result<()> {
    Ok(())
}

pub(super) fn warn_system_install_conflict() {}

fn xml_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
