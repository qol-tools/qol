use crate::plugins::Plugin;
use std::path::PathBuf;

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "macos")]
mod macos;

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod socket_cleanup;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
compile_error!("plugins::daemon_tracker::platform is not implemented for this target OS");

#[cfg(target_os = "linux")]
pub(super) fn pid_exe_path(pid: i32) -> Option<PathBuf> {
    linux::pid_exe_path(pid)
}

#[cfg(target_os = "macos")]
pub(super) fn pid_exe_path(pid: i32) -> Option<PathBuf> {
    macos::pid_exe_path(pid)
}

#[cfg(target_os = "windows")]
pub(super) fn pid_exe_path(_pid: i32) -> Option<PathBuf> {
    None
}

#[cfg(target_os = "linux")]
pub(super) fn kill_orphan_daemons() {
    linux::kill_orphan_daemons();
}

#[cfg(target_os = "macos")]
pub(super) fn kill_orphan_daemons() {
    macos::kill_orphan_daemons();
}

#[cfg(target_os = "windows")]
pub(super) fn kill_orphan_daemons() {}

#[cfg(target_os = "linux")]
pub(super) fn clean_stale_sockets(plugins: &[Plugin]) {
    linux::clean_stale_sockets(plugins);
}

#[cfg(target_os = "macos")]
pub(super) fn clean_stale_sockets(plugins: &[Plugin]) {
    macos::clean_stale_sockets(plugins);
}

#[cfg(target_os = "windows")]
pub(super) fn clean_stale_sockets(_plugins: &[Plugin]) {}
