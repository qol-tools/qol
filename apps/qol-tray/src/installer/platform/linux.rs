use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

const ICON_64: &[u8] = include_bytes!("../../../assets/icons/64.png");
const ICON_128: &[u8] = include_bytes!("../../../assets/icons/128.png");
const ICON_256: &[u8] = include_bytes!("../../../assets/icons/256.png");

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

pub(super) fn register_application(binary_path: &Path) -> Result<()> {
    install_icons()?;
    install_desktop_entry(binary_path)?;
    refresh_caches();
    Ok(())
}

fn install_icons() -> Result<()> {
    let data_dir = dirs::data_dir().context("Could not determine data directory")?;
    let icons = [
        ("64x64", ICON_64),
        ("128x128", ICON_128),
        ("256x256", ICON_256),
    ];
    for (size, data) in icons {
        let icon_dir = data_dir
            .join("icons")
            .join("hicolor")
            .join(size)
            .join("apps");
        std::fs::create_dir_all(&icon_dir)?;
        std::fs::write(icon_dir.join("qol-tray.png"), data)?;
    }
    Ok(())
}

fn install_desktop_entry(binary_path: &Path) -> Result<()> {
    let data_dir = dirs::data_dir().context("Could not determine data directory")?;
    let apps_dir = data_dir.join("applications");
    std::fs::create_dir_all(&apps_dir)?;
    let desktop = render_app_desktop_entry(binary_path);
    std::fs::write(apps_dir.join("qol-tray.desktop"), desktop)?;
    Ok(())
}

fn render_app_desktop_entry(binary_path: &Path) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=QoL Tray\n\
         Comment=Quality of Life Tray daemon\n\
         Exec={}\n\
         Icon=qol-tray\n\
         Terminal=false\n\
         Categories=Utility;\n\
         StartupNotify=false\n",
        binary_path.display()
    )
}

fn refresh_caches() {
    let data_dir = dirs::data_dir();
    if let Some(dir) = &data_dir {
        let _ = std::process::Command::new("update-desktop-database")
            .arg(dir.join("applications"))
            .output();
        let _ = std::process::Command::new("gtk-update-icon-cache")
            .arg(dir.join("icons").join("hicolor"))
            .output();
    }
}

pub(super) fn warn_system_install_conflict() {
    let system_binary = std::path::Path::new("/usr/bin/qol-tray");
    if system_binary.exists() {
        println!(
            "Warning: A system-wide installation exists at {}.\n\
             Run 'sudo apt remove qol-tray' to avoid conflicts.\n\
             The user-local install at ~/.local/bin/ takes precedence if it appears earlier in PATH.",
            system_binary.display()
        );
    }
}
