use std::process::Child;
use std::time::{Duration, Instant};

pub(super) fn is_pid_alive(pid: i32) -> bool {
    // SAFETY: libc::kill with signal 0 performs process existence check only.
    unsafe { libc::kill(pid, 0) == 0 }
}

pub(super) fn terminate_pid(pid: i32, grace: Duration) {
    escalate(pid, pid, grace);
}

pub(super) fn terminate_group(pid: i32, grace: Duration) {
    escalate(pid, -pid, grace);
}

fn escalate(pid: i32, signal_target: i32, grace: Duration) {
    if !is_pid_alive(pid) {
        return;
    }

    unsafe {
        libc::kill(signal_target, libc::SIGTERM);
    }

    std::thread::sleep(grace);

    if !is_pid_alive(pid) {
        return;
    }

    unsafe {
        libc::kill(signal_target, libc::SIGKILL);
    }

    std::thread::sleep(Duration::from_millis(10));
    unsafe {
        libc::waitpid(pid, std::ptr::null_mut(), libc::WNOHANG);
    }
}

pub(super) fn terminate_owned(child: &mut Child, grace: Duration) -> std::io::Result<()> {
    let pid = child.id() as i32;
    unsafe {
        libc::kill(-pid, libc::SIGTERM);
    }

    let deadline = Instant::now() + grace;
    loop {
        if child.try_wait()?.is_some() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    unsafe {
        libc::kill(-pid, libc::SIGKILL);
    }
    child.wait()?;
    Ok(())
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
