use std::time::Duration;

#[cfg(unix)]
mod unix_common;

#[cfg(not(any(unix, target_os = "windows")))]
mod unsupported;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(unix)]
use unix_common as imp;
#[cfg(not(any(unix, target_os = "windows")))]
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
