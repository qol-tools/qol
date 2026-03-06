use std::time::Duration;

#[cfg(unix)]
mod unix_common;

#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(unix, target_os = "windows")))]
compile_error!("process_utils::platform helpers are required for this target OS");

#[cfg(unix)]
pub(super) fn is_pid_alive(pid: i32) -> bool {
    unix_common::is_pid_alive(pid)
}

#[cfg(target_os = "windows")]
pub(super) fn is_pid_alive(pid: i32) -> bool {
    windows::is_pid_alive(pid)
}

#[cfg(unix)]
pub(super) fn terminate_pid(pid: i32, grace: Duration) {
    unix_common::terminate_pid(pid, grace);
}

#[cfg(target_os = "windows")]
pub(super) fn terminate_pid(pid: i32, grace: Duration) {
    windows::terminate_pid(pid, grace);
}

#[cfg(unix)]
pub(super) fn reap_children_nonblocking() {
    unix_common::reap_children_nonblocking();
}

#[cfg(target_os = "windows")]
pub(super) fn reap_children_nonblocking() {
    windows::reap_children_nonblocking();
}
