use super::ManagedProcess;
use crate::plugins::Plugin;
use std::path::{Path, PathBuf};

pub(super) trait DaemonTrackerPlatform {
    fn pid_exe_path(pid: i32) -> Option<PathBuf>;
    fn kill_orphan_daemons();
    fn managed_processes() -> Vec<ManagedProcess>;
    fn clean_stale_sockets(plugins: &[Plugin]);
    fn kill_managed_process(process: &ManagedProcess, roots: &ManagedRoots) -> bool;
    fn is_host_binary(executable: &Path) -> bool {
        let Some(name) = executable.file_name().and_then(|name| name.to_str()) else {
            return false;
        };
        let name = name.strip_suffix(" (deleted)").unwrap_or(name);
        let name = name.strip_suffix(".exe").unwrap_or(name);
        let tray = crate::installer::binary_filename();
        let tray = tray.strip_suffix(".exe").unwrap_or(&tray);
        name == tray || name == "qol" || name == "qol-tray-doctor"
    }
}

#[cfg(not(any(unix, target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(unix)]
mod unix;
#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
mod unix_fallback;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(unix, target_os = "windows")))]
use fallback::Platform;
#[cfg(target_os = "linux")]
use linux::Platform;
#[cfg(target_os = "macos")]
use macos::Platform;
#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
use unix_fallback::Platform;
#[cfg(target_os = "windows")]
use windows::Platform;

#[cfg(not(any(unix, target_os = "windows")))]
pub(crate) use fallback::ManagedRoots;
#[cfg(unix)]
pub(crate) use unix::ManagedRoots;
#[cfg(target_os = "windows")]
pub(crate) use windows::ManagedRoots;

pub(super) fn pid_exe_path(pid: i32) -> Option<PathBuf> {
    Platform::pid_exe_path(pid)
}

pub(super) fn kill_orphan_daemons() {
    Platform::kill_orphan_daemons();
}

pub(super) fn managed_processes() -> Vec<ManagedProcess> {
    Platform::managed_processes()
}

pub(super) fn clean_stale_sockets(plugins: &[Plugin]) {
    Platform::clean_stale_sockets(plugins);
}

pub(super) fn kill_managed_process(process: &ManagedProcess, roots: &ManagedRoots) -> bool {
    Platform::kill_managed_process(process, roots)
}

pub(super) fn is_host_binary(executable: &Path) -> bool {
    Platform::is_host_binary(executable)
}
