use std::io;
use std::os::unix::process::ExitStatusExt;
use std::process::{Child, ExitStatus};
use std::time::{Duration, Instant};

const WAIT_INTERVAL: Duration = Duration::from_millis(50);
const REAP_DELAY: Duration = Duration::from_millis(10);

pub(crate) fn is_pid_alive(pid: u32) -> bool {
    let Ok(pid) = pid_t(pid) else {
        return false;
    };
    if unsafe { libc::kill(pid, 0) } == 0 {
        return true;
    }
    io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

pub(crate) fn signal_term_pid(pid: u32) -> io::Result<()> {
    signal(pid_t(pid)?, libc::SIGTERM)
}

pub(crate) fn kill_pid(pid: u32) -> io::Result<()> {
    signal(pid_t(pid)?, libc::SIGKILL)
}

pub(crate) fn try_wait_pid(pid: u32) -> io::Result<Option<ExitStatus>> {
    let pid = pid_t(pid)?;
    loop {
        let mut status = 0;
        let result = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
        if result == 0 {
            return Ok(None);
        }
        if result == pid {
            return Ok(Some(ExitStatus::from_raw(status)));
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::EINTR) {
            continue;
        }
        return Err(error);
    }
}

pub(crate) fn wait_pid(pid: u32) -> io::Result<ExitStatus> {
    loop {
        if let Some(status) = try_wait_pid(pid)? {
            return Ok(status);
        }
        if !is_pid_alive(pid) {
            return Ok(ExitStatus::from_raw(0));
        }
        std::thread::sleep(WAIT_INTERVAL);
    }
}

pub(crate) fn terminate_pid(pid: u32, grace: Duration) {
    let Ok(pid) = pid_t(pid) else {
        return;
    };
    escalate(pid, pid, grace);
}

pub(crate) fn terminate_group(pid: u32, grace: Duration) {
    let Ok(pid) = pid_t(pid) else {
        return;
    };
    escalate(pid, -pid, grace);
}

fn escalate(pid: libc::pid_t, signal_target: libc::pid_t, grace: Duration) {
    if !is_pid_alive(pid as u32) {
        return;
    }
    let _ = signal(signal_target, libc::SIGTERM);
    std::thread::sleep(grace);
    if is_pid_alive(pid as u32) {
        let _ = signal(signal_target, libc::SIGKILL);
    }
    std::thread::sleep(REAP_DELAY);
    let _ = try_wait_pid(pid as u32);
}

pub(crate) fn terminate_owned(child: &mut Child, grace: Duration) -> io::Result<()> {
    let pid = pid_t(child.id())?;
    let signal_target = owned_signal_target(pid);
    let _ = signal(signal_target, libc::SIGTERM);
    let deadline = Instant::now() + grace;
    loop {
        if child.try_wait()?.is_some() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(WAIT_INTERVAL);
    }
    let _ = signal(signal_target, libc::SIGKILL);
    child.wait()?;
    Ok(())
}

pub(crate) fn reap_children_nonblocking() {
    loop {
        let mut status = 0;
        let result = unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) };
        if result > 0 {
            continue;
        }
        if result < 0 && io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
            continue;
        }
        return;
    }
}

fn pid_t(pid: u32) -> io::Result<libc::pid_t> {
    let pid = libc::pid_t::try_from(pid)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "pid is out of range"))?;
    if pid <= 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "pid must be positive",
        ));
    }
    Ok(pid)
}

fn signal(target: libc::pid_t, signal: libc::c_int) -> io::Result<()> {
    if unsafe { libc::kill(target, signal) } == 0 {
        return Ok(());
    }
    Err(io::Error::last_os_error())
}

fn owned_signal_target(pid: libc::pid_t) -> libc::pid_t {
    if unsafe { libc::getpgid(pid) } == pid {
        return -pid;
    }
    pid
}
