use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const PROTOCOL_VERSION: &str = "2";
const PROTOCOL_ENV: &str = "QOL_PROCESS_GUARDIAN_PROTOCOL";
const OWNER_ENV: &str = "QOL_PROCESS_GUARDIAN_OWNER_FD";
const CLEANUP_ENV: &str = "QOL_PROCESS_GUARDIAN_CLEANUP_FD";
const DISARM_ENV: &str = "QOL_PROCESS_GUARDIAN_DISARM_FD";
const READY_ENV: &str = "QOL_PROCESS_GUARDIAN_READY_FD";
const KILL_ENV: &str = "QOL_PROCESS_GUARDIAN_KILL_FD";
const EVENTS_ENV: &str = "QOL_PROCESS_GUARDIAN_EVENTS_FD";
const START_TIMEOUT: Duration = Duration::from_secs(2);
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);
const EXIT_TIMEOUT: Duration = Duration::from_secs(3);
const WAIT_INTERVAL: Duration = Duration::from_millis(20);

pub(super) struct Guardian {
    creator_pid: libc::pid_t,
    state: Mutex<GuardianState>,
}

struct GuardianState {
    cleanup: Option<OwnedFd>,
    disarm: Option<OwnedFd>,
    child: Option<Child>,
}

struct GuardianEntry {
    owner: OwnedFd,
    cleanup: OwnedFd,
    disarm: OwnedFd,
    ready: OwnedFd,
    kill: OwnedFd,
    events: OwnedFd,
}

enum GuardianDecision {
    Cleanup,
    Disarm,
}

impl Guardian {
    pub(super) fn spawn(mut command: Command, kill: OwnedFd, events: OwnedFd) -> io::Result<Self> {
        let owner = owner_pidfd()?;
        let cleanup = eventfd()?;
        let disarm = eventfd()?;
        let ready = eventfd()?;
        let inherited = [
            owner.as_raw_fd(),
            cleanup.as_raw_fd(),
            disarm.as_raw_fd(),
            ready.as_raw_fd(),
            kill.as_raw_fd(),
            events.as_raw_fd(),
        ];
        configure_guardian_command(&mut command, inherited);
        let mut child = command.spawn()?;
        drop((owner, kill, events));
        if let Err(error) = confirm_guardian_started(&mut child, &ready) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        Ok(Self {
            creator_pid: unsafe { libc::getpid() },
            state: Mutex::new(GuardianState {
                cleanup: Some(cleanup),
                disarm: Some(disarm),
                child: Some(child),
            }),
        })
    }

    pub(super) fn disarm(&self) -> io::Result<()> {
        if !self.is_creator() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "only the process guardian creator may disarm it",
            ));
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("process guardian state is unavailable"))?;
        state.finish(true)
    }

    fn is_creator(&self) -> bool {
        (unsafe { libc::getpid() }) == self.creator_pid
    }
}

impl Drop for Guardian {
    fn drop(&mut self) {
        let is_creator = self.is_creator();
        let state = self
            .state
            .get_mut()
            .unwrap_or_else(|error| error.into_inner());
        if !is_creator {
            state.close_inherited_copy();
            return;
        }
        let _ = state.finish(false);
    }
}

impl GuardianState {
    fn close_inherited_copy(&mut self) {
        self.cleanup.take();
        self.disarm.take();
        self.child.take();
    }

    fn finish(&mut self, disarm: bool) -> io::Result<()> {
        if self.child.is_none() {
            return Ok(());
        }
        let signal_result = if disarm {
            signal_guardian(&self.disarm, "disarm")
        } else {
            signal_guardian(&self.cleanup, "cleanup")
        };
        self.cleanup.take();
        self.disarm.take();
        let wait_result = self.wait_for_exit();
        combine_results(signal_result, wait_result)
    }

    fn wait_for_exit(&mut self) -> io::Result<()> {
        let Some(child) = self.child.as_mut() else {
            return Ok(());
        };
        let deadline = Instant::now()
            .checked_add(EXIT_TIMEOUT)
            .ok_or_else(|| io::Error::other("process guardian exit deadline overflow"))?;
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    self.child.take();
                    if status.success() {
                        return Ok(());
                    }
                    return Err(io::Error::other(format!(
                        "process guardian exited with {status}"
                    )));
                }
                Ok(None) => {}
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
            let now = Instant::now();
            if now >= deadline {
                let Some(mut child) = self.child.take() else {
                    return Ok(());
                };
                let _ = child.kill();
                let _ = child.wait();
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "process guardian did not exit after containment cleanup",
                ));
            }
            std::thread::sleep(WAIT_INTERVAL.min(deadline.duration_since(now)));
        }
    }
}

impl GuardianEntry {
    fn from_environment() -> io::Result<Self> {
        if std::env::var(PROTOCOL_ENV).as_deref() != Ok(PROTOCOL_VERSION) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unsupported process guardian protocol",
            ));
        }
        let descriptors = [
            descriptor_from_environment(OWNER_ENV)?,
            descriptor_from_environment(CLEANUP_ENV)?,
            descriptor_from_environment(DISARM_ENV)?,
            descriptor_from_environment(READY_ENV)?,
            descriptor_from_environment(KILL_ENV)?,
            descriptor_from_environment(EVENTS_ENV)?,
        ];
        let mut distinct = descriptors;
        distinct.sort_unstable();
        if distinct.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "process guardian descriptors must be distinct",
            ));
        }
        validate_pidfd(descriptors[0])?;
        validate_eventfd(descriptors[1])?;
        validate_eventfd(descriptors[2])?;
        validate_eventfd(descriptors[3])?;
        validate_cgroup_control(descriptors[4], libc::O_WRONLY)?;
        validate_cgroup_control(descriptors[5], libc::O_RDONLY)?;
        Ok(unsafe {
            Self {
                owner: OwnedFd::from_raw_fd(descriptors[0]),
                cleanup: OwnedFd::from_raw_fd(descriptors[1]),
                disarm: OwnedFd::from_raw_fd(descriptors[2]),
                ready: OwnedFd::from_raw_fd(descriptors[3]),
                kill: OwnedFd::from_raw_fd(descriptors[4]),
                events: OwnedFd::from_raw_fd(descriptors[5]),
            }
        })
    }

    fn run(self) -> io::Result<()> {
        signal_eventfd(&self.ready)?;
        match wait_for_guardian_decision(&self.disarm, &self.cleanup, &self.owner) {
            Ok(GuardianDecision::Disarm) => Ok(()),
            Ok(GuardianDecision::Cleanup) => {
                kill_until_empty(&self.kill, &self.events, CLEANUP_TIMEOUT)
            }
            Err(protocol) => {
                let cleanup = kill_until_empty(&self.kill, &self.events, CLEANUP_TIMEOUT);
                combine_results(Err(protocol), cleanup)
            }
        }
    }
}

pub(super) fn run_entry() -> io::Result<()> {
    GuardianEntry::from_environment()?.run()
}

fn configure_guardian_command(command: &mut Command, inherited: [RawFd; 6]) {
    command
        .env(PROTOCOL_ENV, PROTOCOL_VERSION)
        .env(OWNER_ENV, inherited[0].to_string())
        .env(CLEANUP_ENV, inherited[1].to_string())
        .env(DISARM_ENV, inherited[2].to_string())
        .env(READY_ENV, inherited[3].to_string())
        .env(KILL_ENV, inherited[4].to_string())
        .env(EVENTS_ENV, inherited[5].to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    unsafe {
        command.pre_exec(move || prepare_guardian_process(&inherited));
    }
}

fn confirm_guardian_started(child: &mut Child, ready: &OwnedFd) -> io::Result<()> {
    wait_eventfd(ready, START_TIMEOUT)?;
    match child.try_wait() {
        Ok(None) => Ok(()),
        Ok(Some(status)) => Err(io::Error::other(format!(
            "process guardian exited during startup with {status}"
        ))),
        Err(error) => Err(error),
    }
}

fn signal_guardian(channel: &Option<OwnedFd>, name: &str) -> io::Result<()> {
    channel
        .as_ref()
        .ok_or_else(|| io::Error::other(format!("process guardian {name} channel is closed")))
        .and_then(signal_eventfd)
}

fn prepare_guardian_process(descriptors: &[RawFd]) -> io::Result<()> {
    loop {
        if unsafe { libc::setsid() } != -1 {
            break;
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::EINTR) {
            return Err(error);
        }
    }
    for descriptor in descriptors {
        loop {
            if unsafe { libc::fcntl(*descriptor, libc::F_SETFD, 0) } != -1 {
                break;
            }
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::EINTR) {
                return Err(error);
            }
        }
    }
    Ok(())
}

fn owner_pidfd() -> io::Result<OwnedFd> {
    let pid = unsafe { libc::getpid() };
    let descriptor = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    if descriptor == -1 {
        return Err(io::Error::last_os_error());
    }
    let descriptor =
        RawFd::try_from(descriptor).map_err(|_| io::Error::other("owner pidfd is out of range"))?;
    Ok(unsafe { OwnedFd::from_raw_fd(descriptor) })
}

fn eventfd() -> io::Result<OwnedFd> {
    let descriptor = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC) };
    if descriptor == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(descriptor) })
}

fn descriptor_from_environment(name: &str) -> io::Result<RawFd> {
    let value = std::env::var(name).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("process guardian has no {name}"),
        )
    })?;
    let descriptor = value.parse::<RawFd>().map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid process guardian descriptor `{value}`: {error}"),
        )
    })?;
    if descriptor <= libc::STDERR_FILENO {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "process guardian descriptors must not replace standard streams",
        ));
    }
    Ok(descriptor)
}

fn validate_pidfd(descriptor: RawFd) -> io::Result<()> {
    let result = unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            descriptor,
            0,
            std::ptr::null::<libc::siginfo_t>(),
            0,
        )
    };
    if result == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        return Ok(());
    }
    Err(error)
}

fn validate_eventfd(descriptor: RawFd) -> io::Result<()> {
    validate_access_mode(descriptor, libc::O_RDWR)
}

fn validate_cgroup_control(descriptor: RawFd, access: libc::c_int) -> io::Result<()> {
    validate_access_mode(descriptor, access)?;
    let mut filesystem: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstatfs(descriptor, &mut filesystem) } == -1 {
        return Err(io::Error::last_os_error());
    }
    if filesystem.f_type != libc::CGROUP2_SUPER_MAGIC {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "process guardian control is not on cgroup v2",
        ));
    }
    Ok(())
}

fn validate_access_mode(descriptor: RawFd, expected: libc::c_int) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
    if flags == -1 {
        return Err(io::Error::last_os_error());
    }
    if flags & libc::O_ACCMODE != expected {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "process guardian descriptor has the wrong access mode",
        ));
    }
    Ok(())
}

fn signal_eventfd(descriptor: &OwnedFd) -> io::Result<()> {
    write_all(descriptor.as_raw_fd(), &1_u64.to_ne_bytes())
}

fn wait_eventfd(descriptor: &OwnedFd, timeout: Duration) -> io::Result<()> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| io::Error::other("process guardian start deadline overflow"))?;
    loop {
        let mut pollfd = libc::pollfd {
            fd: descriptor.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "process guardian did not acknowledge startup",
            ));
        }
        let result = unsafe { libc::poll(&mut pollfd, 1, duration_millis(remaining)) };
        if result > 0 && pollfd.revents & libc::POLLIN != 0 {
            read_eventfd(descriptor.as_raw_fd())?;
            return Ok(());
        }
        if result > 0 {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                format!(
                    "process guardian startup channel returned poll events {:#x}",
                    pollfd.revents
                ),
            ));
        }
        if result == 0 {
            continue;
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::EINTR) {
            return Err(error);
        }
    }
}

fn wait_for_guardian_decision(
    disarm: &OwnedFd,
    cleanup: &OwnedFd,
    owner: &OwnedFd,
) -> io::Result<GuardianDecision> {
    loop {
        let mut descriptors = [
            libc::pollfd {
                fd: disarm.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: cleanup.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: owner.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        let result = unsafe { libc::poll(descriptors.as_mut_ptr(), descriptors.len() as _, -1) };
        if result == -1 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            return Err(error);
        }
        if descriptors[0].revents & libc::POLLIN != 0 {
            read_eventfd(disarm.as_raw_fd())?;
            return Ok(GuardianDecision::Disarm);
        }
        if descriptors[1].revents & libc::POLLIN != 0 {
            read_eventfd(cleanup.as_raw_fd())?;
            return Ok(GuardianDecision::Cleanup);
        }
        if descriptors[2].revents & (libc::POLLIN | libc::POLLHUP) != 0 {
            return Ok(GuardianDecision::Cleanup);
        }
        return Err(unexpected_guardian_events(&descriptors));
    }
}

fn unexpected_guardian_events(descriptors: &[libc::pollfd; 3]) -> io::Error {
    io::Error::other(format!(
        "process guardian channels returned poll events {:#x}/{:#x}/{:#x}",
        descriptors[0].revents, descriptors[1].revents, descriptors[2].revents
    ))
}

fn kill_until_empty(kill: &OwnedFd, events: &OwnedFd, timeout: Duration) -> io::Result<()> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| io::Error::other("process guardian cleanup deadline overflow"))?;
    let mut last_error = None;
    loop {
        if let Err(error) = write_all(kill.as_raw_fd(), b"1") {
            last_error = Some(error);
        }
        match cgroup_populated(events.as_raw_fd()) {
            Ok(false) => return Ok(()),
            Ok(true) => {}
            Err(error) => last_error = Some(error),
        }
        let now = Instant::now();
        if now >= deadline {
            let detail = last_error
                .map(|error| format!("; last error: {error}"))
                .unwrap_or_default();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("process guardian could not empty its cgroup within {timeout:?}{detail}"),
            ));
        }
        std::thread::sleep(WAIT_INTERVAL.min(deadline.duration_since(now)));
    }
}

fn cgroup_populated(descriptor: RawFd) -> io::Result<bool> {
    let mut bytes = [0_u8; 512];
    let read = loop {
        let read = unsafe { libc::pread(descriptor, bytes.as_mut_ptr().cast(), bytes.len(), 0) };
        if read != -1 {
            break usize::try_from(read)
                .map_err(|_| io::Error::other("cgroup event record is too large"))?;
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::EINTR) {
            return Err(error);
        }
    };
    let content = std::str::from_utf8(&bytes[..read])
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    content
        .lines()
        .find_map(|line| line.strip_prefix("populated "))
        .map(|value| value != "0")
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "cgroup.events has no populated state",
            )
        })
}

fn read_eventfd(descriptor: RawFd) -> io::Result<()> {
    let mut bytes = [0_u8; std::mem::size_of::<u64>()];
    let mut offset = 0;
    while offset < bytes.len() {
        let read = unsafe {
            libc::read(
                descriptor,
                bytes[offset..].as_mut_ptr().cast(),
                bytes.len() - offset,
            )
        };
        if read > 0 {
            offset += usize::try_from(read)
                .map_err(|_| io::Error::other("eventfd record is too large"))?;
            continue;
        }
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "eventfd closed before a complete record",
            ));
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::EINTR) {
            return Err(error);
        }
    }
    Ok(())
}

fn write_all(descriptor: RawFd, bytes: &[u8]) -> io::Result<()> {
    let mut offset = 0;
    while offset < bytes.len() {
        let written = unsafe {
            libc::write(
                descriptor,
                bytes[offset..].as_ptr().cast(),
                bytes.len() - offset,
            )
        };
        if written > 0 {
            offset += usize::try_from(written)
                .map_err(|_| io::Error::other("guardian write is too large"))?;
            continue;
        }
        if written == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "guardian descriptor accepted no data",
            ));
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::EINTR) {
            return Err(error);
        }
    }
    Ok(())
}

fn duration_millis(duration: Duration) -> libc::c_int {
    libc::c_int::try_from(duration.as_millis()).unwrap_or(libc::c_int::MAX)
}

fn combine_results(first: io::Result<()>, second: io::Result<()>) -> io::Result<()> {
    match (first, second) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(first), Err(second)) => {
            Err(io::Error::new(second.kind(), format!("{first}; {second}")))
        }
    }
}
