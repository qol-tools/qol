use std::io;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

const WAIT_INTERVAL: Duration = Duration::from_millis(50);
const REAP_DELAY: Duration = Duration::from_millis(10);
static CANCELLATION_SIGNAL_COUNT: AtomicUsize = AtomicUsize::new(0);
static CANCELLATION_INSTALL: OnceLock<Result<(), i32>> = OnceLock::new();

pub(crate) struct CurrentProcessTreeGuard;

impl CurrentProcessTreeGuard {
    pub(crate) fn disarm(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) fn guard_current_process_tree() -> io::Result<CurrentProcessTreeGuard> {
    Ok(CurrentProcessTreeGuard)
}

pub(crate) fn isolate_owned_command(command: &mut Command) -> io::Result<()> {
    command.process_group(0);
    Ok(())
}

pub(crate) fn install_cancellation_handler() -> io::Result<()> {
    let result = CANCELLATION_INSTALL.get_or_init(install_signal_handlers);
    match result {
        Ok(()) => Ok(()),
        Err(code) => Err(io::Error::from_raw_os_error(*code)),
    }
}

pub(crate) fn cancellation_requested() -> bool {
    cancellation_signal_count() > 0
}

pub(crate) fn cancellation_signal_count() -> usize {
    CANCELLATION_SIGNAL_COUNT.load(Ordering::Acquire)
}

fn install_signal_handlers() -> Result<(), i32> {
    for signal in [libc::SIGINT, libc::SIGTERM] {
        let previous = unsafe {
            libc::signal(
                signal,
                cancellation_signal_handler as *const () as libc::sighandler_t,
            )
        };
        if previous == libc::SIG_ERR {
            return Err(io::Error::last_os_error()
                .raw_os_error()
                .unwrap_or(libc::EINVAL));
        }
    }
    Ok(())
}

extern "C" fn cancellation_signal_handler(_: libc::c_int) {
    CANCELLATION_SIGNAL_COUNT.fetch_add(1, Ordering::Release);
}

pub(crate) fn is_pid_alive(pid: u32) -> bool {
    let Ok(pid) = pid_t(pid) else {
        return false;
    };
    signal_target_alive(pid)
}

pub(crate) fn is_group_alive(pid: u32) -> bool {
    let Ok(pid) = pid_t(pid) else {
        return false;
    };
    signal_target_alive(-pid)
}

pub(super) fn signal_target_alive(target: libc::pid_t) -> bool {
    if unsafe { libc::kill(target, 0) } == 0 {
        return true;
    }
    io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

pub(crate) fn signal_term_pid(pid: u32) -> io::Result<()> {
    signal(pid_t(pid)?, libc::SIGTERM)
}

pub(crate) fn signal_term_group(pid: u32) -> io::Result<()> {
    signal(-pid_t(pid)?, libc::SIGTERM)
}

pub(crate) fn kill_pid(pid: u32) -> io::Result<()> {
    signal(pid_t(pid)?, libc::SIGKILL)
}

pub(crate) fn kill_group(pid: u32) -> io::Result<()> {
    signal(-pid_t(pid)?, libc::SIGKILL)
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
    escalate_group(pid, grace);
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

fn escalate_group(pid: libc::pid_t, grace: Duration) {
    let signal_target = -pid;
    if signal_target_alive(signal_target) {
        let _ = signal(signal_target, libc::SIGTERM);
        let deadline = Instant::now() + grace;
        while signal_target_alive(signal_target) && Instant::now() < deadline {
            std::thread::sleep(WAIT_INTERVAL);
        }
        if signal_target_alive(signal_target) {
            let _ = signal(signal_target, libc::SIGKILL);
        }
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

pub(crate) fn spawn_detached(command: &mut Command) -> io::Result<()> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    unsafe {
        command.pre_exec(|| {
            match libc::fork() {
                -1 => return Err(io::Error::last_os_error()),
                0 => {}
                _ => libc::_exit(0),
            }
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut intermediate = command.spawn()?;
    intermediate.wait()?;
    Ok(())
}

pub(super) fn pid_t(pid: u32) -> io::Result<libc::pid_t> {
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
