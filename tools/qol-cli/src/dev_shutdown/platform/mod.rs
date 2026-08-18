use super::TrackedDaemonPid;
use std::process::Command;

pub(crate) trait DevShutdownPlatform {
    fn snapshot_runtime_daemon_pids(&self) -> Vec<TrackedDaemonPid>;
    fn group_is_owned(&self, daemon: &TrackedDaemonPid) -> bool;
    fn configure_tray_child(&self, command: &mut Command) -> std::io::Result<()>;
}

#[cfg(unix)]
mod pid_files;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(crate) use fallback::Platform;
#[cfg(target_os = "linux")]
pub(crate) use linux::Platform;
#[cfg(target_os = "macos")]
pub(crate) use macos::Platform;
#[cfg(target_os = "windows")]
pub(crate) use windows::Platform;
