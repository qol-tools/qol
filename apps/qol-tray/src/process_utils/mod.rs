use std::time::Duration;

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;
#[cfg(not(any(unix, windows)))]
mod unsupported;

#[cfg(unix)]
use unix as imp;
#[cfg(windows)]
use windows as imp;
#[cfg(not(any(unix, windows)))]
use unsupported as imp;

pub fn is_pid_alive(pid: i32) -> bool {
    imp::is_pid_alive(pid)
}

pub fn terminate_pid(pid: i32, grace: Duration) {
    imp::terminate_pid(pid, grace);
}

pub fn reap_children_nonblocking() {
    imp::reap_children_nonblocking();
}
