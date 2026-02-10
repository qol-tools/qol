use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub fn install_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Could not determine home directory")?;
    Ok(home.join(".local").join("bin"))
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
    let parent = path.parent().context("Autostart path has no parent directory")?;
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

fn xml_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
