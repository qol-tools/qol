use crate::plugins::Plugin;
#[cfg(unix)]
use std::path::PathBuf;

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "macos")]
mod macos;

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod socket_cleanup;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
compile_error!("plugins::daemon_tracker::platform is not implemented for this target OS");

#[cfg(unix)]
pub(super) fn pid_exe_path(pid: i32) -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    return linux::pid_exe_path(pid);

    #[cfg(target_os = "macos")]
    return macos::pid_exe_path(pid);
}

pub(super) fn kill_orphan_daemons() {
    #[cfg(target_os = "linux")]
    linux::kill_orphan_daemons();

    #[cfg(target_os = "macos")]
    macos::kill_orphan_daemons();
}

#[cfg(target_os = "linux")]
pub(super) fn managed_processes() -> Vec<super::ManagedProcess> {
    linux::managed_processes()
}

#[cfg(target_os = "macos")]
pub(super) fn managed_processes() -> Vec<super::ManagedProcess> {
    macos::managed_processes()
}

#[cfg(target_os = "windows")]
pub(super) fn managed_processes() -> Vec<super::ManagedProcess> {
    Vec::new()
}

pub(super) fn clean_stale_sockets(plugins: &[Plugin]) {
    #[cfg(target_os = "linux")]
    linux::clean_stale_sockets(plugins);

    #[cfg(target_os = "macos")]
    macos::clean_stale_sockets(plugins);

    #[cfg(target_os = "windows")]
    let _ = plugins;
}
