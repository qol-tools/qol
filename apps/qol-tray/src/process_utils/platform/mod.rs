use std::time::Duration;

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod unix_common;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod unsupported;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
use unsupported as imp;
#[cfg(target_os = "windows")]
use windows as imp;

pub(super) fn is_pid_alive(pid: i32) -> bool {
    imp::is_pid_alive(pid)
}

pub(super) fn terminate_pid(pid: i32, grace: Duration) {
    imp::terminate_pid(pid, grace);
}

pub(super) fn reap_children_nonblocking() {
    imp::reap_children_nonblocking();
}
