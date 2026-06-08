use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use super::AutostartOps;
use crate::installer::desktop_entry::{format_desktop_exec_command, parse_desktop_exec_program};

const DESKTOP_TEMPLATE: &str =
    include_str!("../../../../scripts/installer/platform/linux/desktop/qol-tray.desktop");

pub(crate) struct Platform;

impl AutostartOps for Platform {
    fn read_target(&self) -> Result<Option<PathBuf>> {
        let path = autostart_path_impl()?;
        read_desktop_at(&path)
    }

    fn write_target(&self, binary: &Path) -> Result<()> {
        let path = autostart_path_impl()?;
        write_desktop_to(&path, binary)
    }

    fn autostart_path(&self) -> Result<PathBuf> {
        autostart_path_impl()
    }
}

fn autostart_path_impl() -> Result<PathBuf> {
    let config_dir = dirs::config_dir().context("Could not determine config directory")?;
    Ok(config_dir.join("autostart").join("qol-tray.desktop"))
}

fn write_desktop_to(path: &Path, binary: &Path) -> Result<()> {
    let exec_line = format!("Exec={}", format_desktop_exec_command(binary, &[]));
    let mut rendered = DESKTOP_TEMPLATE
        .lines()
        .map(|line| {
            if line.starts_with("Exec=") {
                exec_line.as_str()
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    rendered.push('\n');
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Autostart path has no parent directory"))?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("Failed to create directory {}", parent.display()))?;
    std::fs::write(path, rendered)
        .with_context(|| format!("Failed to write autostart file {}", path.display()))?;
    Ok(())
}

fn read_desktop_at(path: &Path) -> Result<Option<PathBuf>> {
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    Ok(parse_exec_line(&content).map(PathBuf::from))
}

fn parse_exec_line(content: &str) -> Option<String> {
    content
        .lines()
        .find_map(|line| line.strip_prefix("Exec="))
        .and_then(parse_desktop_exec_program)
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_then_read(desktop_path: &Path, binary: &Path) -> Option<PathBuf> {
        write_desktop_to(desktop_path, binary).unwrap();
        read_desktop_at(desktop_path).unwrap()
    }

    #[test]
    fn round_trip_user_local_bin() {
        let tmp = TempDir::new().unwrap();
        let desktop = tmp.path().join("qol-tray.desktop");
        let binary = PathBuf::from("/home/x/.local/bin/qol-tray");
        assert_eq!(write_then_read(&desktop, &binary), Some(binary));
    }

    #[test]
    fn round_trip_worktree_path_with_spaces() {
        let tmp = TempDir::new().unwrap();
        let desktop = tmp.path().join("qol-tray.desktop");
        let binary = PathBuf::from("/home/x/work tree/qol-tools/qol-tray/target/debug/qol-tray");
        assert_eq!(write_then_read(&desktop, &binary), Some(binary));
    }

    #[test]
    fn read_returns_none_when_no_exec_line() {
        let tmp = TempDir::new().unwrap();
        let desktop = tmp.path().join("qol-tray.desktop");
        std::fs::write(&desktop, "[Desktop Entry]\nName=QoL Tray\n").unwrap();
        assert_eq!(read_desktop_at(&desktop).unwrap(), None);
    }
}
