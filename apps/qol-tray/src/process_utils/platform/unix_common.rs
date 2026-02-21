use std::time::Duration;

pub(super) fn is_pid_alive(pid: i32) -> bool {
    // SAFETY: libc::kill with signal 0 performs process existence check only.
    unsafe { libc::kill(pid, 0) == 0 }
}

pub(super) fn terminate_pid(pid: i32, grace: Duration) {
    if !is_pid_alive(pid) {
        return;
    }

    // SAFETY: Signals are sent to caller-provided pid; errors are ignored,
    // matching previous best-effort semantics.
    unsafe {
        libc::kill(pid, libc::SIGTERM);
    }

    std::thread::sleep(grace);

    if is_pid_alive(pid) {
        // SAFETY: Same reasoning as above for SIGKILL best-effort cleanup.
        unsafe {
            libc::kill(pid, libc::SIGKILL);
        }
    }
}

pub(super) fn reap_children_nonblocking() {
    // SAFETY: waitpid(-1, WNOHANG) reaps any exited child in non-blocking mode.
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
