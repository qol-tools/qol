use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};

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

    fn register_application(&self, _binary_path: &Path) -> Result<()> {
        Ok(())
    }

    fn ensure_desktop_registration(&self) -> Result<()> {
        Ok(())
    }

    fn warn_system_install_conflict(&self) {}

    fn remove_legacy_install(&self) {}
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
    let target = comparable_path(binary_path);
    let own_pid = std::process::id();
    pids_named(exe_name)
        .into_iter()
        .filter(|pid| *pid != own_pid)
        .filter(|pid| image_path(*pid).is_some_and(|image| comparable_path(&image) == target))
        .collect()
}

fn comparable_path(path: &Path) -> String {
    let text = path.to_string_lossy().replace('/', "\\");
    text.strip_prefix(r"\\?\").unwrap_or(&text).to_lowercase()
}

fn pids_named(exe_name: &str) -> Vec<u32> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Vec::new();
    }
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
    let mut pids = Vec::new();
    let mut more = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
    while more {
        let len = entry
            .szExeFile
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(entry.szExeFile.len());
        if String::from_utf16_lossy(&entry.szExeFile[..len]).eq_ignore_ascii_case(exe_name) {
            pids.push(entry.th32ProcessID);
        }
        more = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
    }
    unsafe { CloseHandle(snapshot) };
    pids
}

fn image_path(pid: u32) -> Option<PathBuf> {
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return None;
    }
    let mut buffer = vec![0u16; 32768];
    let mut length = buffer.len() as u32;
    let queried = unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            buffer.as_mut_ptr(),
            &mut length,
        )
    };
    unsafe { CloseHandle(process) };
    (queried != 0).then(|| PathBuf::from(String::from_utf16_lossy(&buffer[..length as usize])))
}
