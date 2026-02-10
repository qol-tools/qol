use std::time::Duration;

#[cfg(unix)]
pub fn is_pid_alive(pid: i32) -> bool {
    unsafe { libc::kill(pid, 0) == 0 }
}

#[cfg(not(unix))]
pub fn is_pid_alive(_pid: i32) -> bool {
    true
}

#[cfg(unix)]
pub fn terminate_pid(pid: i32, grace: Duration) {
    if !is_pid_alive(pid) {
        return;
    }
    unsafe {
        libc::kill(pid, libc::SIGTERM);
    }
    std::thread::sleep(grace);
    if is_pid_alive(pid) {
        unsafe {
            libc::kill(pid, libc::SIGKILL);
        }
    }
}

#[cfg(not(unix))]
pub fn terminate_pid(_pid: i32, _grace: Duration) {}

#[cfg(unix)]
pub fn reap_children_nonblocking() {
    unsafe {
        loop {
            let mut status = 0;
            let reaped = libc::waitpid(-1, &mut status, libc::WNOHANG);
            if reaped <= 0 {
                break;
            }
        }
    }
}

#[cfg(not(unix))]
pub fn reap_children_nonblocking() {}
