pub(crate) mod ico;
mod plugin_release;
mod registration;
mod registry;
pub(in crate::installer) mod run_key;
pub(crate) mod shortcut;

use anyhow::{Context, Result};
use qol_host_fixes::residency::HostResidency;
use qol_platform::windows_path::same_path;
use std::fs;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{
    SetHandleInformation, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::System::Console::{
    GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
};

use self::registration::Layout;
use super::windows_rules::uninstall::{
    deferred_removal_script, plan, run_uninstall, FileRemoval, UninstallPlan, UninstallSteps,
};
use super::InstallerOps;

const SHUTDOWN_WAIT: Duration = Duration::from_secs(10);
const SHUTDOWN_POLL: Duration = Duration::from_millis(100);
const TERMINATE_GRACE: Duration = Duration::from_secs(5);
const DEFERRED_REMOVAL_LOG: &str = "qol-tray-uninstall.log";

pub(super) struct Platform;

impl InstallerOps for Platform {
    fn binary_filename(&self) -> String {
        qol_apps::tray_install::binary_filename().to_string()
    }

    fn install_dir(&self) -> Result<PathBuf> {
        Ok(qol_apps::tray_install::install_dir()?)
    }

    fn start_now(&self, binary_path: &Path) -> Result<()> {
        keep_std_handles_from_children();
        qol_process::spawn_detached(&mut Command::new(binary_path))
            .with_context(|| format!("Failed to start {}", binary_path.display()))
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
        let Some(layout) = installed_layout(&current_exe)? else {
            return Ok(());
        };
        registration::refresh(&layout)
    }

    fn warn_system_install_conflict(&self) {}

    fn remove_legacy_install(&self) {}

    fn uninstall(&self, binary_path: &Path) -> Result<()> {
        let layout = Layout::for_binary(binary_path)?;
        run_uninstall(&mut WindowsUninstall {
            platform: self,
            binary: binary_path,
            layout: &layout,
        })
    }
}

pub(crate) fn refresh_after_update(binary: &Path, installer: Option<&Path>) -> Result<()> {
    let Some(layout) = installed_layout(binary)? else {
        return Ok(());
    };
    if let Some(installer) = installer {
        copy_uninstaller(installer, &layout.uninstaller)?;
    }
    if !layout.shortcut.exists() {
        return Ok(());
    }
    registration::write_shortcut(&layout)
}

fn installed_layout(binary: &Path) -> Result<Option<Layout>> {
    let installed = qol_apps::tray_install::installed_binary()?;
    if !same_path(binary, &installed)
        || !crate::installer::has_install_marker(&installed)
        || !crate::installer::mode::is_production_mode()
    {
        return Ok(None);
    }
    Layout::for_binary(&installed).map(Some)
}

struct WindowsUninstall<'a> {
    platform: &'a Platform,
    binary: &'a Path,
    layout: &'a Layout,
}

impl UninstallSteps for WindowsUninstall<'_> {
    fn end_residency(&mut self) -> Result<()> {
        if !HostResidency::current().is_resident() {
            return Ok(());
        }
        crate::features::resident_policy::apply_residency(HostResidency::Portable)
    }

    fn release_plugins(&mut self) -> Result<()> {
        plugin_release::release_all()
    }

    fn stop_running(&mut self) -> Result<()> {
        self.platform.stop_running(self.binary)
    }

    fn remove_autostart(&mut self) -> Result<()> {
        run_key::remove()
    }

    fn remove_shell_entries(&mut self) -> Result<()> {
        registration::remove_shell_entries(self.layout)
    }

    fn remove_toast_identity(&mut self) -> Result<()> {
        qol_plugin_daemon::notification::platform::uninstall_toast_identity()
            .context("Failed to remove the QoL Tray notification identity")
    }

    fn remove_files(&mut self) -> Result<FileRemoval> {
        let install_dir = qol_apps::tray_install::install_dir()?;
        let current =
            std::env::current_exe().context("Failed to determine the uninstaller path")?;
        match plan(self.binary, &install_dir, &current)? {
            UninstallPlan::RemoveRoot(root) => fs::remove_dir_all(&root)
                .or_else(|error| match error.kind() {
                    std::io::ErrorKind::NotFound => Ok(()),
                    _ => Err(error),
                })
                .with_context(|| format!("Failed to remove {}", root.display()))
                .map(|()| FileRemoval::Done),
            UninstallPlan::RemoveAfterExit(root) => {
                registration::remove_file_if_present(self.binary)?;
                if let Some(marker) = crate::paths::install_marker::marker_path(self.binary) {
                    registration::remove_file_if_present(&marker)?;
                }
                remove_after_exit(&root)?;
                Ok(FileRemoval::Deferred)
            }
        }
    }

    fn remove_uninstall_entry(&mut self) -> Result<()> {
        registration::remove_uninstall_entry()
    }
}

fn keep_std_handles_from_children() {
    for id in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        let handle = unsafe { GetStdHandle(id) };
        if !handle.is_null() && handle != INVALID_HANDLE_VALUE {
            unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) };
        }
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
    copy_uninstaller(&current, destination)
}

fn copy_uninstaller(source: &Path, destination: &Path) -> Result<()> {
    let staged = destination.with_extension("new");
    fs::copy(source, &staged).with_context(|| {
        format!(
            "Failed to copy the uninstaller {} to {}",
            source.display(),
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

fn remove_after_exit(root: &Path) -> Result<()> {
    let log = std::env::temp_dir().join(DEFERRED_REMOVAL_LOG);
    let reg = qol_process::system_program_path("reg.exe");
    let Some(script) = deferred_removal_script(
        &root.display().to_string(),
        &log.display().to_string(),
        &reg.display().to_string(),
        registration::UNINSTALL_KEY,
    ) else {
        anyhow::bail!(
            "{} was left in place: its path cannot be quoted for cmd.exe",
            root.display()
        );
    };
    let mut command = qol_process::system_command("cmd.exe");
    qol_process::hide_console_window(&mut command)
        .raw_arg(script)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("Failed to schedule removal of {}", root.display()))?;
    log::warn!(
        "{} is removed after this uninstaller exits; the result is written to {}",
        root.display(),
        log.display()
    );
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
