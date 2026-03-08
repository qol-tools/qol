use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

const DESKTOP_TEMPLATE: &str =
    include_str!("../../../scripts/installer/platform/linux/desktop/qol-tray.desktop");

pub(super) fn install_dir() -> Result<PathBuf> {
    super::unix_common::install_dir()
}

pub(super) fn autostart_path() -> Result<PathBuf> {
    let config_dir = dirs::config_dir().context("Could not determine config directory")?;
    Ok(config_dir.join("autostart").join("qol-tray.desktop"))
}

pub(super) fn write_autostart_entry(binary_path: &Path) -> Result<()> {
    let path = autostart_path()?;
    let desktop = render_desktop_entry(binary_path);
    super::write_text_file(&path, &desktop)
}

fn render_desktop_entry(binary_path: &Path) -> String {
    let exec_line = format!("Exec={}", binary_path.display());
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
    rendered
}

pub(super) fn start_now(binary_path: &Path) -> Result<()> {
    if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
        println!("Skipping auto-start because no GUI session was detected.");
        return Ok(());
    }

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

pub(super) fn copy_symlink(source: &Path, target: &Path) -> Result<()> {
    super::unix_common::copy_symlink(source, target)
}

pub(super) fn on_file_copied(source: &Path, target: &Path) -> Result<()> {
    super::unix_common::on_file_copied(source, target)
}
