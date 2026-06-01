use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use super::AutostartOps;

pub(crate) struct Platform;

impl AutostartOps for Platform {
    fn read_target(&self) -> Result<Option<PathBuf>> {
        let path = autostart_path_impl()?;
        read_cmd_at(&path)
    }

    fn write_target(&self, binary: &Path) -> Result<()> {
        let path = autostart_path_impl()?;
        write_cmd_to(&path, binary)
    }

    fn autostart_path(&self) -> Result<PathBuf> {
        autostart_path_impl()
    }
}

fn autostart_path_impl() -> Result<PathBuf> {
    let app_data = std::env::var_os("APPDATA").context("APPDATA is not set")?;
    Ok(PathBuf::from(app_data)
        .join("Microsoft")
        .join("Windows")
        .join("Start Menu")
        .join("Programs")
        .join("Startup")
        .join("qol-tray.cmd"))
}

fn write_cmd_to(path: &Path, binary: &Path) -> Result<()> {
    let escaped = binary.display().to_string().replace('"', "\"\"");
    let content = format!("@echo off\r\nstart \"\" \"{escaped}\"\r\n");
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Autostart path has no parent directory"))?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("Failed to create directory {}", parent.display()))?;
    std::fs::write(path, content)
        .with_context(|| format!("Failed to write autostart file {}", path.display()))?;
    Ok(())
}

fn read_cmd_at(path: &Path) -> Result<Option<PathBuf>> {
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    Ok(parse_start_line(&content).map(PathBuf::from))
}

fn parse_start_line(content: &str) -> Option<String> {
    let start = content.find("start \"\" \"")? + "start \"\" \"".len();
    let rest = &content[start..];
    let end = rest.rfind('"')?;
    let raw = &rest[..end];
    Some(raw.replace("\"\"", "\""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_then_read(cmd_path: &Path, binary: &Path) -> Option<PathBuf> {
        write_cmd_to(cmd_path, binary).unwrap();
        read_cmd_at(cmd_path).unwrap()
    }

    #[test]
    fn round_trip_install_path() {
        let tmp = TempDir::new().unwrap();
        let cmd = tmp.path().join("qol-tray.cmd");
        let binary = PathBuf::from(r"C:\Users\x\AppData\Local\Programs\qol-tray\bin\qol-tray.exe");
        assert_eq!(write_then_read(&cmd, &binary), Some(binary));
    }

    #[test]
    fn round_trip_path_with_space() {
        let tmp = TempDir::new().unwrap();
        let cmd = tmp.path().join("qol-tray.cmd");
        let binary = PathBuf::from(r"C:\Users\x y\repos\qol-tray\target\debug\qol-tray.exe");
        assert_eq!(write_then_read(&cmd, &binary), Some(binary));
    }
}
