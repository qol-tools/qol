use std::io;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const WAIT_INTERVAL: Duration = Duration::from_millis(50);
const REAP_DELAY: Duration = Duration::from_millis(10);
static CANCELLATION_REQUESTED: AtomicBool = AtomicBool::new(false);
static CANCELLATION_INSTALL: OnceLock<Result<(), i32>> = OnceLock::new();

pub(crate) struct ProcessTreeGuard {
    target: Mutex<Option<ProcessTreeTarget>>,
}

#[derive(Clone, Copy)]
enum ProcessTreeTarget {
    Process(libc::pid_t),
    ProcessGroup(libc::pid_t),
}

impl ProcessTreeGuard {
    pub(crate) fn assign(&self, child: &Child) -> io::Result<()> {
        let pid = pid_t(child.id())?;
        let process_group = unsafe { libc::getpgid(pid) };
        if process_group == -1 {
            return Err(io::Error::last_os_error());
        }
        let mut target = self
            .target
            .lock()
            .map_err(|_| io::Error::other("process-tree assignment state is unavailable"))?;
        if target.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "process tree already owns a process",
            ));
        }
        *target = Some(if process_group == pid {
            ProcessTreeTarget::ProcessGroup(process_group)
        } else {
            ProcessTreeTarget::Process(pid)
        });
        Ok(())
    }

    pub(crate) fn terminate_and_wait(&self, timeout: Duration) -> io::Result<()> {
        let target = self
            .target
            .lock()
            .map_err(|_| io::Error::other("process-tree assignment state is unavailable"))?
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotConnected,
                    "process tree has no assigned process",
                )
            })?;
        let started = Instant::now();
        let deadline = started.checked_add(timeout).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "process-tree timeout is too large",
            )
        })?;
        match target {
            ProcessTreeTarget::Process(pid) => terminate_process(pid, started, deadline, timeout),
            ProcessTreeTarget::ProcessGroup(process_group) => {
                terminate_process_group(process_group, started, deadline, timeout)
            }
        }
    }
}

pub(crate) struct CurrentProcessTreeGuard;

impl CurrentProcessTreeGuard {
    pub(crate) fn disarm(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) fn guard_current_process_tree() -> io::Result<CurrentProcessTreeGuard> {
    Ok(CurrentProcessTreeGuard)
}

fn terminate_process_group(
    process_group: libc::pid_t,
    started: Instant,
    deadline: Instant,
    timeout: Duration,
) -> io::Result<()> {
    let graceful_deadline = started.checked_add(timeout / 2).unwrap_or(deadline);
    signal_group(process_group, libc::SIGTERM)?;
    if wait_for_group_exit(process_group, graceful_deadline) {
        return Ok(());
    }
    signal_group(process_group, libc::SIGKILL)?;
    if wait_for_group_exit(process_group, deadline) {
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        format!("process group {process_group} did not exit within {timeout:?}"),
    ))
}

fn terminate_process(
    pid: libc::pid_t,
    started: Instant,
    deadline: Instant,
    timeout: Duration,
) -> io::Result<()> {
    let graceful_deadline = started.checked_add(timeout / 2).unwrap_or(deadline);
    signal_process(pid, libc::SIGTERM)?;
    if wait_for_process_exit(pid, graceful_deadline) {
        return Ok(());
    }
    signal_process(pid, libc::SIGKILL)?;
    if wait_for_process_exit(pid, deadline) {
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        format!("process {pid} did not exit within {timeout:?}"),
    ))
}

pub(crate) fn own_current_process_tree() -> io::Result<ProcessTreeGuard> {
    Ok(ProcessTreeGuard {
        target: Mutex::new(None),
    })
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
    CANCELLATION_REQUESTED.load(Ordering::Acquire)
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
    CANCELLATION_REQUESTED.store(true, Ordering::Release);
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

#[cfg(target_os = "linux")]
pub(crate) fn is_pid_zombie(pid: u32) -> bool {
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return false;
    };
    stat.rsplit_once(") ")
        .and_then(|(_, fields)| fields.chars().next())
        == Some('Z')
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn is_pid_zombie(_pid: u32) -> bool {
    false
}

#[cfg(target_os = "linux")]
pub(crate) fn process_identity(pid: u32) -> io::Result<String> {
    let pid = pid_t(pid)?;
    let boot_id = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")?
        .trim()
        .to_string();
    if boot_id.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Linux boot id is empty",
        ));
    }
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))?;
    let fields = stat
        .rsplit_once(") ")
        .map(|(_, fields)| fields)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid Linux process stat"))?;
    let start_ticks = fields
        .split_whitespace()
        .nth(19)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing process start time"))?;
    start_ticks.parse::<u64>().map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid process start time: {error}"),
        )
    })?;
    Ok(format!("linux:{boot_id}:{start_ticks}"))
}

#[cfg(target_os = "macos")]
pub(crate) fn process_identity(pid: u32) -> io::Result<String> {
    let pid = pid_t(pid)?;
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>();
    let read = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            std::ptr::from_mut(&mut info).cast(),
            i32::try_from(size).map_err(|_| io::Error::other("process info is too large"))?,
        )
    };
    if read != i32::try_from(size).unwrap_or(i32::MAX) {
        return Err(io::Error::last_os_error());
    }
    Ok(format!(
        "macos:{}:{}",
        info.pbi_start_tvsec, info.pbi_start_tvusec
    ))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(crate) fn process_identity(_pid: u32) -> io::Result<String> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "durable process identity is unsupported on this Unix platform",
    ))
}

fn signal_target_alive(target: libc::pid_t) -> bool {
    if unsafe { libc::kill(target, 0) } == 0 {
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
    if !signal_target_alive(signal_target) {
        return;
    }
    let _ = signal(signal_target, libc::SIGTERM);
    let deadline = Instant::now() + grace;
    while signal_target_alive(signal_target) && Instant::now() < deadline {
        std::thread::sleep(WAIT_INTERVAL);
    }
    if signal_target_alive(signal_target) {
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

fn signal_group(process_group: libc::pid_t, signal_number: libc::c_int) -> io::Result<()> {
    let result = signal(-process_group, signal_number);
    match result {
        Err(error) if error.raw_os_error() == Some(libc::ESRCH) => Ok(()),
        other => other,
    }
}

fn wait_for_group_exit(process_group: libc::pid_t, deadline: Instant) -> bool {
    loop {
        if !signal_target_alive(-process_group) {
            return true;
        }
        let now = Instant::now();
        if now >= deadline {
            return false;
        }
        std::thread::sleep(WAIT_INTERVAL.min(deadline.duration_since(now)));
    }
}

fn signal_process(pid: libc::pid_t, signal_number: libc::c_int) -> io::Result<()> {
    let result = signal(pid, signal_number);
    match result {
        Err(error) if error.raw_os_error() == Some(libc::ESRCH) => Ok(()),
        other => other,
    }
}

fn wait_for_process_exit(pid: libc::pid_t, deadline: Instant) -> bool {
    loop {
        if !signal_target_alive(pid) {
            return true;
        }
        let now = Instant::now();
        if now >= deadline {
            return false;
        }
        std::thread::sleep(WAIT_INTERVAL.min(deadline.duration_since(now)));
    }
}

fn owned_signal_target(pid: libc::pid_t) -> libc::pid_t {
    if unsafe { libc::getpgid(pid) } == pid {
        return -pid;
    }
    pid
}
