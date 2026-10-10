mod registration;
mod registry;
pub(in crate::installer) mod run_key;
mod shortcut;

use anyhow::{Context, Result};
use qol_platform::windows_path::same_path;
use std::fs;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use self::registration::Layout;
use super::windows_rules::uninstall::{plan, UninstallPlan};
use super::InstallerOps;

const SHUTDOWN_WAIT: Duration = Duration::from_secs(10);
const SHUTDOWN_POLL: Duration = Duration::from_millis(100);
const TERMINATE_GRACE: Duration = Duration::from_secs(5);

pub(super) struct Platform;

impl InstallerOps for Platform {
    fn binary_filename(&self) -> String {
        qol_apps::tray_install::binary_filename().to_string()
    }

    fn install_dir(&self) -> Result<PathBuf> {
        Ok(qol_apps::tray_install::install_dir()?)
    }

    fn start_now(&self, binary_path: &Path) -> Result<()> {
        super::spawn_detached(binary_path)
    }

    fn stop_running(&self, binary_path: &Path) -> Result<()> {
        if installed_pids(binary_path).is_empty() {
            return Ok(());
        }
        request_graceful_shutdown();
        if wait_for_exit(binary_path) {
            return Ok(());
        }
        for pid in installed_pids(binary_path) {
            log::warn!("installed tray pid {pid} ignored the shutdown request, terminating it");
            qol_process::terminate_pid(pid, TERMINATE_GRACE);
        }
        Ok(())
    }

    fn set_executable_permissions(&self, _path: &Path) -> Result<()> {
        Ok(())
    }

    fn prepare_atomic_replace(&self, installed_binary: &Path) -> Result<()> {
        if !installed_binary.exists() {
            return Ok(());
        }
        fs::remove_file(installed_binary).with_context(|| {
            format!(
                "Failed to remove existing installed binary {}",
                installed_binary.display()
            )
        })
    }

    fn should_bootstrap_current_install(&self, _binary_path: &Path) -> Result<bool> {
        Ok(false)
    }

    fn register_application(&self, binary_path: &Path) -> Result<()> {
        let layout = Layout::for_binary(binary_path)?;
        install_uninstaller(&layout.uninstaller)?;
        registration::register(&layout)
    }

    fn ensure_desktop_registration(&self) -> Result<()> {
        let Ok(current_exe) = std::env::current_exe() else {
            return Ok(());
        };
        let installed = qol_apps::tray_install::installed_binary()?;
        if !same_path(&current_exe, &installed)
            || !crate::installer::has_install_marker(&installed)
            || !crate::installer::mode::is_production_mode()
        {
            return Ok(());
        }
        registration::refresh(&Layout::for_binary(&installed)?)
    }

    fn warn_system_install_conflict(&self) {}

    fn remove_legacy_install(&self) {}

    fn uninstall(&self, binary_path: &Path) -> Result<()> {
        let layout = Layout::for_binary(binary_path)?;
        self.stop_running(binary_path)?;
        run_key::remove()?;
        registration::remove(&layout)?;
        remove_install_files(binary_path)
    }
}

fn install_uninstaller(destination: &Path) -> Result<()> {
    let current = std::env::current_exe().context("Failed to determine the installer path")?;
    let is_installer = current
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case(&registration::uninstaller_filename()));
    if !is_installer || same_path(&current, destination) {
        return Ok(());
    }
    let staged = destination.with_extension("new");
    fs::copy(&current, &staged).with_context(|| {
        format!(
            "Failed to copy the uninstaller {} to {}",
            current.display(),
            staged.display()
        )
    })?;
    fs::rename(&staged, destination).with_context(|| {
        format!(
            "Failed to install the uninstaller {}",
            destination.display()
        )
    })
}

fn remove_install_files(binary_path: &Path) -> Result<()> {
    let install_dir = qol_apps::tray_install::install_dir()?;
    let current = std::env::current_exe().context("Failed to determine the uninstaller path")?;
    match plan(binary_path, &install_dir, &current)? {
        UninstallPlan::RemoveRoot(root) => fs::remove_dir_all(&root)
            .or_else(|error| match error.kind() {
                std::io::ErrorKind::NotFound => Ok(()),
                _ => Err(error),
            })
            .with_context(|| format!("Failed to remove {}", root.display())),
        UninstallPlan::RemoveAfterExit(root) => {
            registration::remove_file_if_present(binary_path)?;
            if let Some(marker) = crate::paths::install_marker::marker_path(binary_path) {
                registration::remove_file_if_present(&marker)?;
            }
            remove_after_exit(&root)
        }
    }
}

fn remove_after_exit(root: &Path) -> Result<()> {
    let root = root.display().to_string();
    if root.contains(['"', '%', '!']) {
        log::warn!("leaving {root} in place: its path cannot be quoted for cmd.exe");
        return Ok(());
    }
    let mut command = Command::new("cmd.exe");
    qol_process::hide_console_window(&mut command)
        .raw_arg(format!(
            "/d /v:off /c ping -n 3 127.0.0.1 >nul & rmdir /s /q \"{root}\""
        ))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("Failed to schedule removal of {root}"))?;
    Ok(())
}

fn request_graceful_shutdown() {
    let route = format!("/api{}", qol_conventions::SHUTDOWN_ROUTE);
    match qol_plugin_api::host_exec::post_to_daemon(&route, "{}") {
        Ok((status, _)) if (200..300).contains(&status) => {}
        Ok((status, _)) => log::warn!("tray shutdown request failed with HTTP {status}"),
        Err(error) => log::warn!("tray shutdown request failed: {error}"),
    }
}

fn wait_for_exit(binary_path: &Path) -> bool {
    let deadline = Instant::now() + SHUTDOWN_WAIT;
    while Instant::now() < deadline {
        if installed_pids(binary_path).is_empty() {
            return true;
        }
        std::thread::sleep(SHUTDOWN_POLL);
    }
    installed_pids(binary_path).is_empty()
}

fn installed_pids(binary_path: &Path) -> Vec<u32> {
    let Some(exe_name) = binary_path.file_name().and_then(|name| name.to_str()) else {
        return Vec::new();
    };
    let own_pid = std::process::id();
    let Ok(processes) = qol_process::processes() else {
        return Vec::new();
    };
    processes
        .into_iter()
        .filter(|entry| entry.pid != own_pid && entry.exe.eq_ignore_ascii_case(exe_name))
        .map(|entry| entry.pid)
        .filter(|pid| {
            qol_process::process_image_path(*pid).is_ok_and(|image| same_path(&image, binary_path))
        })
        .collect()
}
