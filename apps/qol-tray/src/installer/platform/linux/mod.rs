use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use self::desktop_entry::{format_desktop_exec_command, DesktopExecArg};
use super::InstallerOps;

pub(in crate::installer) mod desktop_entry;

const ICON_64: &[u8] = include_bytes!("../../../../assets/icons/64.png");
const ICON_128: &[u8] = include_bytes!("../../../../assets/icons/128.png");
const ICON_256: &[u8] = include_bytes!("../../../../assets/icons/256.png");

pub(super) struct Platform;

impl InstallerOps for Platform {
    fn binary_filename(&self) -> String {
        "qol-tray".to_string()
    }

    fn install_dir(&self) -> Result<PathBuf> {
        let home = dirs::home_dir().context("Could not determine home directory")?;
        Ok(home.join(".local").join("bin"))
    }

    fn start_now(&self, binary_path: &Path) -> Result<()> {
        if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
            println!("Skipping auto-start because no GUI session was detected.");
            return Ok(());
        }
        if super::unix_common::is_running("qol-tray") {
            return Ok(());
        }
        super::unix_common::start_now(binary_path)
    }

    fn stop_running(&self, binary_path: &Path) -> Result<()> {
        if !binary_path.exists() {
            return super::unix_common::stop_running_by_name("qol-tray");
        }
        stop_running_binary(binary_path);
        Ok(())
    }

    fn set_executable_permissions(&self, path: &Path) -> Result<()> {
        super::unix_common::set_executable_permissions(path)
    }

    fn prepare_atomic_replace(&self, _installed_binary: &Path) -> Result<()> {
        Ok(())
    }

    fn should_bootstrap_current_install(&self, _binary_path: &Path) -> Result<bool> {
        Ok(false)
    }

    fn register_application(&self, binary_path: &Path) -> Result<()> {
        install_icons()?;
        install_desktop_entry(binary_path)?;
        refresh_caches();
        Ok(())
    }

    fn warn_system_install_conflict(&self) {
        let system_binary = Path::new("/usr/bin/qol-tray");
        if !system_binary.exists() {
            return;
        }
        println!(
            "Warning: A system-wide installation exists at {}.\n\
             Run 'sudo apt remove qol-tray' to avoid conflicts.\n\
             The user-local install at ~/.local/bin/ takes precedence if it appears earlier in PATH.",
            system_binary.display()
        );
    }

    fn remove_legacy_install(&self) {}
}

fn stop_running_binary(binary_path: &Path) {
    let pids = pids_for_binary(binary_path);
    if pids.is_empty() {
        return;
    }
    for pid in &pids {
        crate::process_utils::terminate_pid(*pid, std::time::Duration::from_millis(100));
    }
    if wait_all_pids_exit(&pids) {
        return;
    }
    for pid in pids {
        crate::process_utils::terminate_pid(pid, std::time::Duration::from_millis(10));
    }
}

fn wait_all_pids_exit(pids: &[i32]) -> bool {
    for _ in 0..30 {
        if pids
            .iter()
            .all(|pid| !crate::process_utils::is_pid_alive(*pid))
        {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    false
}

fn pid_matches_binary(pid: i32, target: &Path) -> bool {
    let exe = Path::new("/proc").join(pid.to_string()).join("exe");
    let Ok(exe_path) = std::fs::read_link(exe) else {
        return false;
    };
    std::fs::canonicalize(&exe_path)
        .unwrap_or(exe_path)
        .as_path()
        == target
}

fn pids_for_binary(binary_path: &Path) -> Vec<i32> {
    let target = std::fs::canonicalize(binary_path).unwrap_or_else(|_| binary_path.to_path_buf());
    let current_pid = std::process::id() as i32;
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_name().to_string_lossy().parse::<i32>().ok())
        .filter(|&pid| pid > 0 && pid != current_pid)
        .filter(|&pid| pid_matches_binary(pid, &target))
        .collect()
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
    let exec = format_desktop_exec_command(binary_path, &[DesktopExecArg::Url]);
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=QoL Tray\n\
         Comment=Quality of Life Tray daemon\n\
         Exec={}\n\
         Icon=qol-tray\n\
         Terminal=false\n\
         Categories=Utility;\n\
         MimeType=x-scheme-handler/qol;\n\
         StartupNotify=false\n",
        exec
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
    let _ = std::process::Command::new("xdg-mime")
        .args(["default", "qol-tray.desktop", "x-scheme-handler/qol"])
        .output();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_entry_registers_qol_scheme_and_passes_url() {
        let entry = render_app_desktop_entry(Path::new("/home/u/.local/bin/qol-tray"));
        assert!(entry.contains("MimeType=x-scheme-handler/qol;"));
        assert!(entry
            .lines()
            .any(|line| line == "Exec=\"/home/u/.local/bin/qol-tray\" %u"));
        assert!(entry.contains("Type=Application"));
    }
}
