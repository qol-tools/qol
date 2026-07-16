use std::cmp::Ordering as CmpOrdering;
use std::collections::BTreeSet;
use std::io;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::{PlatformSpawnFailure, PreparedSpawnCleanup};

#[cfg(target_os = "linux")]
#[path = "linux_guardian.rs"]
mod linux_guardian;

#[cfg(target_os = "linux")]
use std::ffi::CString;
#[cfg(target_os = "linux")]
use std::fs::{File, OpenOptions};
#[cfg(target_os = "linux")]
use std::io::{Read, Write};
#[cfg(target_os = "linux")]
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
#[cfg(target_os = "linux")]
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
#[cfg(target_os = "linux")]
use std::sync::atomic::AtomicBool;
#[cfg(target_os = "linux")]
use std::sync::{Arc, Condvar};

const WAIT_INTERVAL: Duration = Duration::from_millis(50);
const REAP_DELAY: Duration = Duration::from_millis(10);
#[cfg(target_os = "linux")]
const PREPARED_ABORT_TIMEOUT: Duration = Duration::from_millis(250);
#[cfg(target_os = "linux")]
const MAX_CGROUP_DEPTH: usize = 64;
#[cfg(target_os = "linux")]
const MAX_CGROUP_NODES: usize = 4096;
#[cfg(target_os = "linux")]
const STALE_RECOVERY_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(target_os = "linux")]
const MAX_STALE_RECOVERY_RECORDS: usize = 256;
#[cfg(target_os = "linux")]
const MAX_STALE_RECOVERY_WORK: usize = 8192;
static CANCELLATION_SIGNAL_COUNT: AtomicUsize = AtomicUsize::new(0);
static CANCELLATION_INSTALL: OnceLock<Result<(), i32>> = OnceLock::new();
static NEXT_PROCESS_GENERATION: AtomicU64 = AtomicU64::new(1);
#[cfg(target_os = "linux")]
static NEXT_CGROUP_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) struct ProcessTreeGuard {
    target: Mutex<Option<ProcessTreeTarget>>,
    #[cfg(target_os = "linux")]
    guardian: Option<linux_guardian::Guardian>,
    #[cfg(target_os = "linux")]
    cgroup: LinuxCgroup,
    #[cfg(target_os = "linux")]
    prepared: AtomicBool,
}

#[cfg(target_os = "linux")]
pub(crate) struct PreparedSpawn {
    acknowledgement: OwnedFd,
    failed_child_reaper: FailedChildReaper,
}

#[cfg(not(target_os = "linux"))]
pub(crate) struct PreparedSpawn;

#[derive(Clone, Debug)]
struct ProcessTreeTarget {
    root_pid: libc::pid_t,
    root: Option<OwnedProcess>,
    scope: ProcessScope,
}

#[derive(Clone, Debug)]
struct OwnedProcess {
    pid: libc::pid_t,
    identity: String,
    generation: u64,
    #[cfg(target_os = "linux")]
    handle: Arc<LinuxProcessHandle>,
}

impl PartialEq for OwnedProcess {
    fn eq(&self, other: &Self) -> bool {
        self.generation == other.generation
    }
}

impl Eq for OwnedProcess {}

impl PartialOrd for OwnedProcess {
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(other))
    }
}

impl Ord for OwnedProcess {
    fn cmp(&self, other: &Self) -> CmpOrdering {
        self.generation.cmp(&other.generation)
    }
}

#[cfg(target_os = "linux")]
#[derive(Debug)]
struct LinuxProcessHandle {
    fd: OwnedFd,
}

#[cfg(target_os = "linux")]
#[derive(Debug)]
struct LinuxCgroup {
    creator_pid: libc::pid_t,
    path: std::path::PathBuf,
    directory: OwnedFd,
    journal_path: std::path::PathBuf,
    _journal_lock: File,
    sealed: AtomicBool,
}

#[cfg(target_os = "linux")]
struct LinuxCgroupAllocation {
    parent: std::path::PathBuf,
    journal_root: std::path::PathBuf,
    namespace: String,
    timestamp: u128,
    _registry_lock: File,
}

#[cfg(target_os = "linux")]
struct CgroupJournal {
    boot_id: String,
    path: std::path::PathBuf,
    device: u64,
    inode: u64,
}

#[cfg(target_os = "linux")]
struct CgroupRecoveryBudget {
    deadline: Instant,
    records_remaining: usize,
    work_remaining: usize,
}

#[cfg(target_os = "linux")]
struct RecoverableCgroup {
    _journal: File,
    journal_path: std::path::PathBuf,
    record: CgroupJournal,
    directory: OwnedFd,
}

#[cfg(target_os = "linux")]
enum RecoverableCgroupState {
    Pending,
    Cleaned,
    Ready(RecoverableCgroup),
}

#[cfg(target_os = "linux")]
struct FailedChildReaper {
    state: Arc<(Mutex<FailedChildReaperState>, Condvar)>,
}

#[cfg(target_os = "linux")]
struct FailedChildReaperState {
    child: Option<Child>,
    closed: bool,
}

#[cfg(target_os = "linux")]
impl CgroupJournal {
    fn encode(&self) -> String {
        format!(
            "version=1\nboot_id={}\npath={}\ndevice={}\ninode={}\n",
            self.boot_id,
            self.path.display(),
            self.device,
            self.inode
        )
    }

    fn parse(content: &str) -> io::Result<Self> {
        let value = |name: &str| {
            content
                .lines()
                .find_map(|line| line.strip_prefix(&format!("{name}=")))
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("cgroup journal has no {name}"),
                    )
                })
        };
        if value("version")? != "1" {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unsupported cgroup journal version",
            ));
        }
        Ok(Self {
            boot_id: value("boot_id")?.to_string(),
            path: std::path::PathBuf::from(value("path")?),
            device: value("device")?.parse().map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("invalid cgroup journal device: {error}"),
                )
            })?,
            inode: value("inode")?.parse().map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("invalid cgroup journal inode: {error}"),
                )
            })?,
        })
    }
}

#[cfg(target_os = "linux")]
impl CgroupRecoveryBudget {
    fn standard() -> io::Result<Self> {
        let deadline = Instant::now()
            .checked_add(STALE_RECOVERY_TIMEOUT)
            .ok_or_else(|| io::Error::other("stale cgroup recovery deadline overflow"))?;
        Ok(Self::new(
            deadline,
            MAX_STALE_RECOVERY_RECORDS,
            MAX_STALE_RECOVERY_WORK,
        ))
    }

    fn new(deadline: Instant, records_remaining: usize, work_remaining: usize) -> Self {
        Self {
            deadline,
            records_remaining,
            work_remaining,
        }
    }

    fn claim_record(&mut self) -> bool {
        if self.records_remaining == 0 || !self.claim_work() {
            return false;
        }
        self.records_remaining -= 1;
        true
    }

    fn claim_work(&mut self) -> bool {
        if self.work_remaining == 0 || Instant::now() >= self.deadline {
            return false;
        }
        self.work_remaining -= 1;
        true
    }

    fn remaining(&self) -> Option<Duration> {
        (self.work_remaining > 0)
            .then(|| self.deadline.saturating_duration_since(Instant::now()))
            .filter(|remaining| !remaining.is_zero())
    }

    fn can_start_pass(&self) -> bool {
        self.records_remaining > 0 && self.remaining().is_some()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
enum ProcessScope {
    Process(libc::pid_t),
    #[cfg(not(target_os = "linux"))]
    ProcessGroup(libc::pid_t),
    #[cfg(not(target_os = "linux"))]
    Session(libc::pid_t),
}

impl ProcessTreeGuard {
    pub(crate) fn prepare_command(&self, command: &mut Command) -> io::Result<PreparedSpawn> {
        #[cfg(target_os = "linux")]
        {
            if self.prepared.swap(true, Ordering::AcqRel) {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "process tree already prepared a command",
                ));
            }
            self.cgroup.prepare_command(command)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = command;
            Err(unsupported_process_tree_containment())
        }
    }

    pub(crate) fn spawn_prepared(
        &self,
        command: &mut Command,
        prepared: PreparedSpawn,
    ) -> Result<Child, PlatformSpawnFailure> {
        #[cfg(target_os = "linux")]
        {
            let child = match command.spawn() {
                Ok(child) => child,
                Err(error) => {
                    return Err(prepared_spawn_failure(error, self.abort_prepared_result()));
                }
            };
            if let Err(error) = self.assign_prepared_child(&child, &prepared) {
                return Err(prepared_spawn_failure(
                    error,
                    self.abort_spawned_child(child, prepared.failed_child_reaper),
                ));
            }
            Ok(child)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (command, prepared);
            Err(PlatformSpawnFailure {
                source: unsupported_process_tree_containment(),
                cleanup: PreparedSpawnCleanup::NotStarted,
            })
        }
    }

    pub(crate) fn abort_prepared(&self) {
        #[cfg(target_os = "linux")]
        let _ = self.abort_prepared_result();
    }

    #[cfg(target_os = "linux")]
    fn assign_prepared_child(&self, child: &Child, prepared: &PreparedSpawn) -> io::Result<()> {
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
        let pid = pid_t(child.id())?;
        let acknowledged = prepared.acknowledged_pid()?;
        if acknowledged != pid {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "prepared command acknowledged PID {acknowledged}, spawned child is PID {pid}"
                ),
            ));
        }
        let root = capture_owned_process(pid)?;
        if !self.cgroup.contains_recursive(pid)?
            && root
                .as_ref()
                .is_some_and(|process| owned_process_alive(process).unwrap_or(true))
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("owned process root PID {pid} escaped its pre-exec cgroup"),
            ));
        }
        *target = Some(ProcessTreeTarget {
            root_pid: pid,
            root,
            scope: ProcessScope::Process(pid),
        });
        Ok(())
    }

    #[cfg(target_os = "linux")]
    fn abort_prepared_result(&self) -> io::Result<()> {
        self.cgroup.force_kill_and_seal(PREPARED_ABORT_TIMEOUT)?;
        self.disarm_guardian()
    }

    #[cfg(target_os = "linux")]
    fn abort_spawned_child(
        &self,
        mut child: Child,
        failed_child_reaper: FailedChildReaper,
    ) -> io::Result<()> {
        let _ = child.kill();
        let cgroup_result = self.abort_prepared_result();
        let deadline = Instant::now() + PREPARED_ABORT_TIMEOUT;
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() < deadline => std::thread::sleep(WAIT_INTERVAL),
                Ok(None) => {
                    failed_child_reaper.handoff(child);
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "failed prepared child did not become reapable",
                    ));
                }
                Err(error) => {
                    failed_child_reaper.handoff(child);
                    return Err(error);
                }
            }
        }
        cgroup_result
    }

    pub(crate) fn terminate_and_wait(&self, timeout: Duration) -> io::Result<()> {
        let target = self
            .target
            .lock()
            .map_err(|_| io::Error::other("process-tree assignment state is unavailable"))?
            .clone()
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
        #[cfg(target_os = "linux")]
        {
            terminate_cgroup_scope(&target, &self.cgroup, started, deadline, timeout)?;
            self.disarm_guardian()
        }
        #[cfg(not(target_os = "linux"))]
        {
            terminate_process_scope(&target, started, deadline, timeout)
        }
    }

    pub(crate) fn recover_pending_spawn(&self, timeout: Duration) -> io::Result<()> {
        #[cfg(target_os = "linux")]
        {
            self.cgroup.force_kill_and_seal(timeout)?;
            self.disarm_guardian()
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = timeout;
            Err(unsupported_process_tree_containment())
        }
    }

    pub(crate) fn terminate_root_and_wait(&self, timeout: Duration) -> io::Result<()> {
        let target = self
            .target
            .lock()
            .map_err(|_| io::Error::other("process-tree assignment state is unavailable"))?
            .clone()
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotConnected,
                    "process tree has no assigned process",
                )
            })?;
        let Some(root) = target.root.as_ref() else {
            return Ok(());
        };
        terminate_owned_process(root, target.scope, timeout)
    }

    pub(crate) fn root_has_exited(&self) -> io::Result<bool> {
        let target = self
            .target
            .lock()
            .map_err(|_| io::Error::other("process-tree assignment state is unavailable"))?
            .clone()
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotConnected,
                    "process tree has no assigned process",
                )
            })?;
        target.root.as_ref().map_or(Ok(true), |root| {
            owned_process_alive(root).map(|alive| !alive)
        })
    }

    #[cfg(target_os = "linux")]
    fn disarm_guardian(&self) -> io::Result<()> {
        self.guardian
            .as_ref()
            .map_or(Ok(()), |guardian| guardian.disarm())
    }
}

#[cfg(not(target_os = "linux"))]
fn unsupported_process_tree_containment() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "verified process-tree containment is unavailable on this Unix platform",
    )
}

#[cfg(target_os = "linux")]
fn prepared_spawn_failure(error: io::Error, cleanup: io::Result<()>) -> PlatformSpawnFailure {
    match cleanup {
        Ok(()) => PlatformSpawnFailure {
            source: error,
            cleanup: PreparedSpawnCleanup::Verified,
        },
        Err(cleanup) => PlatformSpawnFailure {
            source: io::Error::new(
                error.kind(),
                format!("{error}; failed to clean the prepared process tree: {cleanup}"),
            ),
            cleanup: PreparedSpawnCleanup::RecoveryPending,
        },
    }
}

#[cfg(target_os = "linux")]
impl PreparedSpawn {
    fn acknowledged_pid(&self) -> io::Result<libc::pid_t> {
        let mut bytes = [0_u8; std::mem::size_of::<libc::pid_t>()];
        let mut offset = 0;
        while offset < bytes.len() {
            let read = unsafe {
                libc::read(
                    self.acknowledgement.as_raw_fd(),
                    bytes[offset..].as_mut_ptr().cast(),
                    bytes.len() - offset,
                )
            };
            if read > 0 {
                offset += usize::try_from(read)
                    .map_err(|_| io::Error::other("prepared PID acknowledgement is too large"))?;
                continue;
            }
            if read == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "prepared command did not acknowledge its process id",
                ));
            }
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            return Err(error);
        }
        Ok(libc::pid_t::from_ne_bytes(bytes))
    }
}

#[cfg(target_os = "linux")]
impl FailedChildReaper {
    fn start() -> io::Result<Self> {
        let state = Arc::new((
            Mutex::new(FailedChildReaperState {
                child: None,
                closed: false,
            }),
            Condvar::new(),
        ));
        let worker = Arc::clone(&state);
        std::thread::Builder::new()
            .name("qol-process-failed-spawn-reaper".to_string())
            .spawn(move || {
                let (state, ready) = &*worker;
                let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
                while state.child.is_none() && !state.closed {
                    state = ready.wait(state).unwrap_or_else(|error| error.into_inner());
                }
                let Some(mut child) = state.child.take() else {
                    return;
                };
                drop(state);
                loop {
                    match child.wait() {
                        Ok(_) => return,
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                        Err(_) => return,
                    }
                }
            })?;
        Ok(Self { state })
    }

    fn handoff(self, child: Child) {
        let (state, ready) = &*self.state;
        let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
        state.child = Some(child);
        state.closed = true;
        ready.notify_one();
        drop(state);
    }
}

#[cfg(target_os = "linux")]
impl Drop for FailedChildReaper {
    fn drop(&mut self) {
        let (state, ready) = &*self.state;
        let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
        state.closed = true;
        ready.notify_one();
    }
}

#[cfg(target_os = "linux")]
fn raw_preexec_error() -> io::Error {
    let code = unsafe { *libc::__errno_location() };
    io::Error::from_raw_os_error(code)
}

#[cfg(target_os = "macos")]
fn raw_preexec_error() -> io::Error {
    let code = unsafe { *libc::__error() };
    io::Error::from_raw_os_error(code)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn raw_preexec_error() -> io::Error {
    io::Error::last_os_error()
}

#[cfg(target_os = "linux")]
fn raw_preexec_write_all(descriptor: libc::c_int, bytes: &[u8]) -> io::Result<()> {
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
            offset += usize::try_from(written).unwrap_or(bytes.len());
            continue;
        }
        if written == 0 {
            return Err(io::Error::from_raw_os_error(libc::EIO));
        }
        let error = raw_preexec_error();
        if error.raw_os_error() == Some(libc::EINTR) {
            continue;
        }
        return Err(error);
    }
    Ok(())
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

#[cfg(target_os = "linux")]
fn terminate_cgroup_scope(
    target: &ProcessTreeTarget,
    cgroup: &LinuxCgroup,
    started: Instant,
    deadline: Instant,
    timeout: Duration,
) -> io::Result<()> {
    if cgroup.sealed.load(Ordering::Acquire) {
        return Ok(());
    }
    let graceful_deadline = started.checked_add(timeout / 2).unwrap_or(deadline);
    let graceful = terminate_cgroup_members(target, cgroup, graceful_deadline);
    let forced = cgroup.force_kill_and_seal_until(deadline, timeout);
    combine_cgroup_shutdown(graceful, forced)
}

#[cfg(target_os = "linux")]
fn terminate_cgroup_members(
    target: &ProcessTreeTarget,
    cgroup: &LinuxCgroup,
    deadline: Instant,
) -> io::Result<()> {
    let mut observed = BTreeSet::new();
    let mut terminated = BTreeSet::new();
    loop {
        observed.extend(cgroup_members(cgroup, &observed)?);
        signal_new_members(target, &observed, libc::SIGTERM, &mut terminated)?;
        if Instant::now() >= deadline {
            break;
        }
        if !cgroup.populated()? {
            break;
        }
        std::thread::sleep(WAIT_INTERVAL.min(deadline.saturating_duration_since(Instant::now())));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn combine_cgroup_shutdown(graceful: io::Result<()>, forced: io::Result<()>) -> io::Result<()> {
    match (graceful.err(), forced) {
        (_, Ok(())) => Ok(()),
        (None, Err(error)) => Err(error),
        (Some(graceful), Err(forced)) => Err(io::Error::new(
            forced.kind(),
            format!("graceful process-tree shutdown failed: {graceful}; forced cleanup failed: {forced}"),
        )),
    }
}

#[cfg(target_os = "linux")]
fn cgroup_members(
    cgroup: &LinuxCgroup,
    known: &BTreeSet<OwnedProcess>,
) -> io::Result<Vec<OwnedProcess>> {
    let initial = cgroup.members_recursive()?;
    let mut captured = Vec::new();
    for pid in initial {
        if let Some(process) = known
            .iter()
            .find(|process| process.pid == pid && owned_process_alive(process).unwrap_or(false))
        {
            captured.push(process.clone());
            continue;
        }
        if let Some(process) = capture_owned_process(pid)? {
            captured.push(process);
        }
    }
    let verified = cgroup
        .members_recursive()?
        .into_iter()
        .collect::<BTreeSet<_>>();
    captured.retain(|process| verified.contains(&process.pid));
    Ok(captured)
}

#[cfg(not(target_os = "linux"))]
fn terminate_process_scope(
    target: &ProcessTreeTarget,
    started: Instant,
    deadline: Instant,
    timeout: Duration,
) -> io::Result<()> {
    if !matches!(target.scope, ProcessScope::Process(_)) {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "verified process-tree containment is unavailable on this Unix platform",
        ));
    }
    let graceful_deadline = started.checked_add(timeout / 2).unwrap_or(deadline);
    let initial = scope_members(target)?;
    if initial.is_empty() {
        return Ok(());
    }
    let mut observed = initial.into_iter().collect::<BTreeSet<_>>();
    let mut terminated = BTreeSet::new();
    signal_new_members(target, &observed, libc::SIGTERM, &mut terminated)?;
    if wait_for_scope_exit(
        target,
        graceful_deadline,
        libc::SIGTERM,
        &mut observed,
        &mut terminated,
    )? {
        return Ok(());
    }
    let mut killed = BTreeSet::new();
    observed.extend(scope_members(target)?);
    signal_new_members(target, &observed, libc::SIGKILL, &mut killed)?;
    if wait_for_scope_exit(target, deadline, libc::SIGKILL, &mut observed, &mut killed)? {
        return Ok(());
    }
    Err(surviving_scope_error(target, observed, timeout))
}

#[cfg(not(target_os = "linux"))]
fn surviving_scope_error(
    target: &ProcessTreeTarget,
    observed: BTreeSet<OwnedProcess>,
    timeout: Duration,
) -> io::Error {
    let survivors = observed
        .into_iter()
        .filter_map(|process| {
            owned_process_alive(&process)
                .unwrap_or(true)
                .then_some(process)
        })
        .map(|process| process.pid.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    io::Error::new(
        io::ErrorKind::TimedOut,
        format!(
            "owned process scope {:?} retained PID(s) {survivors} after {timeout:?}",
            target.scope
        ),
    )
}

fn terminate_owned_process(
    process: &OwnedProcess,
    scope: ProcessScope,
    timeout: Duration,
) -> io::Result<()> {
    let started = Instant::now();
    let deadline = started.checked_add(timeout).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "process termination timeout is too large",
        )
    })?;
    let graceful_deadline = started.checked_add(timeout / 2).unwrap_or(deadline);
    if !signal_owned_process(process, scope, libc::SIGTERM)? {
        return Ok(());
    }
    if wait_for_owned_process_exit(process, graceful_deadline)? {
        return Ok(());
    }
    if signal_owned_process(process, scope, libc::SIGKILL)?
        && wait_for_owned_process_exit(process, deadline)?
    {
        return Ok(());
    }
    if !owned_process_alive(process)? {
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        format!(
            "owned process root PID {} did not exit within {timeout:?}",
            process.pid
        ),
    ))
}

fn wait_for_owned_process_exit(process: &OwnedProcess, deadline: Instant) -> io::Result<bool> {
    loop {
        if !owned_process_alive(process)? {
            return Ok(true);
        }
        let now = Instant::now();
        if now >= deadline {
            return Ok(false);
        }
        std::thread::sleep(WAIT_INTERVAL.min(deadline.duration_since(now)));
    }
}

#[cfg(not(target_os = "linux"))]
fn wait_for_scope_exit(
    target: &ProcessTreeTarget,
    deadline: Instant,
    signal_number: libc::c_int,
    observed: &mut BTreeSet<OwnedProcess>,
    signaled: &mut BTreeSet<OwnedProcess>,
) -> io::Result<bool> {
    loop {
        observed.extend(scope_members(target)?);
        let alive = observed
            .iter()
            .filter_map(|process| match owned_process_alive(process) {
                Ok(true) => Some(Ok(process.clone())),
                Ok(false) => None,
                Err(error) => Some(Err(error)),
            })
            .collect::<io::Result<BTreeSet<_>>>()?;
        if alive.is_empty() {
            return Ok(true);
        }
        signal_new_members(target, &alive, signal_number, signaled)?;
        let now = Instant::now();
        if now >= deadline {
            return Ok(false);
        }
        std::thread::sleep(WAIT_INTERVAL.min(deadline.duration_since(now)));
    }
}

fn signal_new_members(
    target: &ProcessTreeTarget,
    members: &BTreeSet<OwnedProcess>,
    signal_number: libc::c_int,
    signaled: &mut BTreeSet<OwnedProcess>,
) -> io::Result<()> {
    let mut members = members.iter().cloned().collect::<Vec<_>>();
    members.sort_by_key(|process| process.pid == target.root_pid);
    for process in members {
        if signaled.contains(&process) {
            continue;
        }
        if signal_owned_process(&process, target.scope, signal_number)? {
            signaled.insert(process);
        }
    }
    Ok(())
}

fn signal_owned_process(
    process: &OwnedProcess,
    scope: ProcessScope,
    signal_number: libc::c_int,
) -> io::Result<bool> {
    if !owned_process_alive(process)? {
        return Ok(false);
    }
    let _still_in_scope = process_in_scope(process.pid, scope)?;
    signal_owned_process_handle(process, signal_number)
}

#[cfg(not(target_os = "linux"))]
fn scope_members(target: &ProcessTreeTarget) -> io::Result<Vec<OwnedProcess>> {
    verify_scope_generation(target)?;
    let root = target.root.as_ref().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotConnected,
            "numeric process scope has no captured root generation",
        )
    })?;
    match target.scope {
        ProcessScope::Process(_) => {
            if owned_process_alive(root)? {
                return Ok(vec![root.clone()]);
            }
            Ok(Vec::new())
        }
        ProcessScope::ProcessGroup(_) | ProcessScope::Session(_) => {
            let mut members = Vec::new();
            for pid in list_process_ids()? {
                if !process_in_scope(pid, target.scope)? {
                    continue;
                }
                let Some(process) = capture_owned_process(pid)? else {
                    continue;
                };
                if !process_in_scope(pid, target.scope)? {
                    continue;
                }
                members.push(process);
            }
            verify_scope_generation(target)?;
            Ok(members)
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn verify_scope_generation(target: &ProcessTreeTarget) -> io::Result<()> {
    let root = target.root.as_ref().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotConnected,
            "numeric process scope has no captured root generation",
        )
    })?;
    if !owned_process_alive(root)? {
        return Ok(());
    }
    if !process_in_scope(root.pid, target.scope)? {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "owned process root PID {} left scope {:?}",
                root.pid, target.scope
            ),
        ));
    }
    Ok(())
}

fn current_process_identity(pid: libc::pid_t) -> io::Result<Option<String>> {
    let pid_u32 = u32::try_from(pid)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "pid is out of range"))?;
    match process_identity(pid_u32) {
        Ok(identity) => Ok(Some(identity)),
        Err(_) if !signal_target_alive(pid) => Ok(None),
        Err(error) => Err(error),
    }
}

fn next_process_generation() -> io::Result<u64> {
    NEXT_PROCESS_GENERATION
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |generation| {
            generation.checked_add(1)
        })
        .map_err(|_| io::Error::other("owned process generation counter exhausted"))
}

#[cfg(target_os = "linux")]
fn capture_owned_process(pid: libc::pid_t) -> io::Result<Option<OwnedProcess>> {
    let handle = match LinuxProcessHandle::open(pid) {
        Ok(handle) => Arc::new(handle),
        Err(error) if error.raw_os_error() == Some(libc::ESRCH) => return Ok(None),
        Err(error) => return Err(error),
    };
    if !handle.is_alive()? {
        return Ok(None);
    }
    if let Err(error) = handle.verify_pid(pid) {
        if !handle.is_alive()? {
            return Ok(None);
        }
        return Err(error);
    }
    let Some(identity) = current_process_identity(pid)? else {
        return Ok(None);
    };
    if let Err(error) = handle.verify_pid(pid) {
        if !handle.is_alive()? {
            return Ok(None);
        }
        return Err(error);
    }
    if !handle.is_alive()? {
        return Ok(None);
    }
    Ok(Some(OwnedProcess {
        pid,
        identity,
        generation: next_process_generation()?,
        handle,
    }))
}

#[cfg(not(target_os = "linux"))]
fn capture_owned_process(pid: libc::pid_t) -> io::Result<Option<OwnedProcess>> {
    let Some(identity) = current_process_identity(pid)? else {
        return Ok(None);
    };
    Ok(Some(OwnedProcess {
        pid,
        identity,
        generation: next_process_generation()?,
    }))
}

#[cfg(target_os = "linux")]
fn owned_process_alive(process: &OwnedProcess) -> io::Result<bool> {
    if !process.handle.is_alive()? {
        return Ok(false);
    }
    if let Err(error) = process.handle.verify_pid(process.pid) {
        if !process.handle.is_alive()? {
            return Ok(false);
        }
        return Err(error);
    }
    let identity = current_process_identity(process.pid)?;
    match identity.as_deref() {
        Some(identity) if identity == process.identity => Ok(true),
        Some(_) => Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "pidfd generation for PID {} no longer matches its captured process identity",
                process.pid
            ),
        )),
        None => Ok(false),
    }
}

#[cfg(not(target_os = "linux"))]
fn owned_process_alive(process: &OwnedProcess) -> io::Result<bool> {
    Ok(current_process_identity(process.pid)?.as_deref() == Some(process.identity.as_str()))
}

#[cfg(target_os = "linux")]
fn signal_owned_process_handle(
    process: &OwnedProcess,
    signal_number: libc::c_int,
) -> io::Result<bool> {
    if !owned_process_alive(process)? {
        return Ok(false);
    }
    process.handle.send_signal(signal_number)
}

#[cfg(not(target_os = "linux"))]
fn signal_owned_process_handle(
    process: &OwnedProcess,
    _signal_number: libc::c_int,
) -> io::Result<bool> {
    if !owned_process_alive(process)? {
        return Ok(false);
    }
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "atomic identity-bound process signaling is unsupported on this Unix platform",
    ))
}

#[cfg(target_os = "linux")]
impl LinuxProcessHandle {
    fn open(pid: libc::pid_t) -> io::Result<Self> {
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
        if fd == -1 {
            return Err(io::Error::last_os_error());
        }
        let fd = i32::try_from(fd).map_err(|_| io::Error::other("pidfd is out of range"))?;
        Ok(Self {
            fd: unsafe { OwnedFd::from_raw_fd(fd) },
        })
    }

    fn verify_pid(&self, expected_pid: libc::pid_t) -> io::Result<()> {
        let path = format!("/proc/self/fdinfo/{}", self.fd.as_raw_fd());
        let content = std::fs::read_to_string(&path)?;
        let pid = content
            .lines()
            .find_map(|line| line.strip_prefix("Pid:"))
            .map(str::trim)
            .and_then(|value| value.parse::<libc::pid_t>().ok())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "pidfd has no process id"))?;
        if pid != expected_pid {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("pidfd targets PID {pid}, expected PID {expected_pid}"),
            ));
        }
        Ok(())
    }

    fn is_alive(&self) -> io::Result<bool> {
        let mut descriptor = libc::pollfd {
            fd: self.fd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let result = loop {
            let result = unsafe { libc::poll(&mut descriptor, 1, 0) };
            if result != -1 {
                break result;
            }
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::EINTR) {
                return Err(error);
            }
        };
        if result == 0 {
            return Ok(true);
        }
        if descriptor.revents & libc::POLLNVAL != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "pidfd became invalid",
            ));
        }
        if descriptor.revents & (libc::POLLIN | libc::POLLHUP) != 0 {
            return Ok(false);
        }
        Err(io::Error::other(format!(
            "pidfd returned unexpected poll events {:#x}",
            descriptor.revents
        )))
    }

    fn send_signal(&self, signal_number: libc::c_int) -> io::Result<bool> {
        let result = unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                self.fd.as_raw_fd(),
                signal_number,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            )
        };
        if result == 0 {
            return Ok(true);
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            return Ok(false);
        }
        Err(error)
    }
}

#[cfg(target_os = "linux")]
impl LinuxCgroup {
    fn create() -> io::Result<Self> {
        let allocation = LinuxCgroupAllocation::start()?;
        for _ in 0..128 {
            if let Some(cgroup) = Self::allocate(&allocation)? {
                return Ok(cgroup);
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "failed to allocate a unique qol process cgroup",
        ))
    }

    fn allocate(allocation: &LinuxCgroupAllocation) -> io::Result<Option<Self>> {
        let nonce = allocation.next_nonce();
        let Some((journal_path, mut journal)) = allocation.create_journal(&nonce)? else {
            return Ok(None);
        };
        let Some((path, directory)) = allocation.create_directory(&nonce, &journal_path)? else {
            return Ok(None);
        };
        let (device, inode) = directory_identity(&directory)?;
        let record = CgroupJournal {
            boot_id: linux_boot_id()?,
            path: path.clone(),
            device,
            inode,
        };
        journal.set_len(0)?;
        journal.write_all(record.encode().as_bytes())?;
        journal.sync_all()?;
        let cgroup = Self {
            creator_pid: unsafe { libc::getpid() },
            path,
            directory,
            journal_path,
            _journal_lock: journal,
            sealed: AtomicBool::new(false),
        };
        cgroup.validate_controls()?;
        Ok(Some(cgroup))
    }

    fn validate_controls(&self) -> io::Result<()> {
        for (control, flags) in [
            ("cgroup.procs", libc::O_RDONLY),
            ("cgroup.events", libc::O_RDONLY),
            ("cgroup.kill", libc::O_WRONLY),
        ] {
            self.open_control(control, flags)?;
        }
        Ok(())
    }

    fn prepare_command(&self, command: &mut Command) -> io::Result<PreparedSpawn> {
        self.open_control("cgroup.procs", libc::O_WRONLY)?;
        let failed_child_reaper = FailedChildReaper::start()?;
        let directory = self.directory.try_clone()?;
        let (acknowledgement, acknowledge) = cloexec_pipe()?;
        let expected_parent = unsafe { libc::getpid() };
        unsafe {
            command
                .pre_exec(move || attach_preexec_cgroup(&directory, expected_parent, &acknowledge));
        }
        Ok(PreparedSpawn {
            acknowledgement,
            failed_child_reaper,
        })
    }

    fn contains_recursive(&self, pid: libc::pid_t) -> io::Result<bool> {
        Ok(self.members_recursive()?.contains(&pid))
    }

    fn members(&self) -> io::Result<Vec<libc::pid_t>> {
        parse_cgroup_processes(&self.read_control("cgroup.procs")?)
    }

    fn members_recursive(&self) -> io::Result<Vec<libc::pid_t>> {
        let mut members = self.members()?;
        collect_descendant_members(&self.path, &mut members)?;
        members.sort_unstable();
        members.dedup();
        Ok(members)
    }

    fn populated(&self) -> io::Result<bool> {
        cgroup_populated(self.directory.as_raw_fd())
    }

    fn kill(&self) -> io::Result<()> {
        self.write_control("cgroup.kill", b"1")
    }

    fn guardian_controls(&self) -> io::Result<(OwnedFd, OwnedFd)> {
        Ok((
            self.open_control("cgroup.kill", libc::O_WRONLY)?,
            self.open_control("cgroup.events", libc::O_RDONLY)?,
        ))
    }

    fn open_control(&self, name: &str, flags: libc::c_int) -> io::Result<OwnedFd> {
        open_at(self.directory.as_raw_fd(), name, flags)
    }

    fn read_control(&self, name: &str) -> io::Result<String> {
        let descriptor = self.open_control(name, libc::O_RDONLY)?;
        let mut file = File::from(descriptor);
        let mut content = String::new();
        file.read_to_string(&mut content)?;
        Ok(content)
    }

    fn write_control(&self, name: &str, content: &[u8]) -> io::Result<()> {
        let descriptor = self.open_control(name, libc::O_WRONLY)?;
        let mut file = File::from(descriptor);
        file.write_all(content)
    }

    fn force_kill_and_seal(&self, timeout: Duration) -> io::Result<()> {
        let deadline = Instant::now().checked_add(timeout).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "cgroup timeout is too large")
        })?;
        self.force_kill_and_seal_until(deadline, timeout)
    }

    fn force_kill_and_seal_until(&self, deadline: Instant, timeout: Duration) -> io::Result<()> {
        let mut last_error = None;
        loop {
            if let Err(error) = self.kill() {
                last_error = Some(error);
            }
            match self.populated() {
                Ok(false) => match self.seal_if_empty() {
                    Ok(true) => return Ok(()),
                    Ok(false) => {}
                    Err(error) => last_error = Some(error),
                },
                Ok(true) => {}
                Err(error) => last_error = Some(error),
            }
            let now = Instant::now();
            if now >= deadline {
                let detail = last_error
                    .map(|error| format!("; last cleanup error: {error}"))
                    .unwrap_or_default();
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!(
                        "owned cgroup {} remained populated or unsealed after {timeout:?}{detail}",
                        self.path.display()
                    ),
                ));
            }
            std::thread::sleep(WAIT_INTERVAL.min(deadline.duration_since(now)));
        }
    }

    fn remove_if_empty(&self) -> io::Result<bool> {
        self.seal_if_empty()
    }

    fn seal_if_empty(&self) -> io::Result<bool> {
        if self.sealed.load(Ordering::Acquire) {
            return Ok(true);
        }
        if self.populated()? {
            return Ok(false);
        }
        if !remove_descendant_cgroups(&self.path)? {
            return Ok(false);
        }
        if self.populated()? {
            return Ok(false);
        }
        verify_open_directory_path(&self.directory, &self.path)?;
        match std::fs::remove_dir(&self.path) {
            Ok(()) => {}
            Err(error) if retryable_cgroup_removal(&error) => return Ok(false),
            Err(error) => return Err(error),
        }
        self.sealed.store(true, Ordering::Release);
        match std::fs::remove_file(&self.journal_path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        Ok(true)
    }
}

#[cfg(target_os = "linux")]
impl LinuxCgroupAllocation {
    fn start() -> io::Result<Self> {
        let root = stable_cgroup_root()?;
        let journal_root = cgroup_journal_root()?;
        let mut recovery_budget = CgroupRecoveryBudget::standard()?;
        let registry_lock = lock_cgroup_registry(&journal_root, &recovery_budget)?;
        recover_stale_cgroups(&root, &journal_root, &mut recovery_budget)?;
        let parent = owned_cgroup_parent(&root, &journal_root)?;
        let namespace = journal_namespace(&journal_root)?;
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| {
                io::Error::other(format!("system clock precedes Unix epoch: {error}"))
            })?
            .as_nanos();
        Ok(Self {
            parent,
            journal_root,
            namespace,
            timestamp,
            _registry_lock: registry_lock,
        })
    }

    fn next_nonce(&self) -> String {
        let sequence = NEXT_CGROUP_ID.fetch_add(1, Ordering::Relaxed);
        format!(
            "{}-{}-{}-{sequence}",
            self.namespace,
            std::process::id(),
            self.timestamp
        )
    }

    fn create_journal(&self, nonce: &str) -> io::Result<Option<(std::path::PathBuf, File)>> {
        let path = self.journal_root.join(format!("{nonce}.lock"));
        let journal = match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(&path)
        {
            Ok(journal) => journal,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Ok(None),
            Err(error) => return Err(error),
        };
        lock_journal(&journal)?;
        Ok(Some((path, journal)))
    }

    fn create_directory(
        &self,
        nonce: &str,
        journal_path: &std::path::Path,
    ) -> io::Result<Option<(std::path::PathBuf, OwnedFd)>> {
        let path = self.parent.join(format!("qol-process-v1-{nonce}"));
        match std::fs::create_dir(&path) {
            Ok(()) => {}
            Err(error) => {
                let _ = std::fs::remove_file(journal_path);
                if error.kind() == io::ErrorKind::AlreadyExists {
                    return Ok(None);
                }
                return Err(error);
            }
        }
        let directory = match open_directory(&path) {
            Ok(directory) => directory,
            Err(error) => {
                let _ = std::fs::remove_dir(&path);
                let _ = std::fs::remove_file(journal_path);
                return Err(error);
            }
        };
        Ok(Some((path, directory)))
    }
}

#[cfg(target_os = "linux")]
fn cloexec_pipe() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut pipe = [0; 2];
    if unsafe { libc::pipe2(pipe.as_mut_ptr(), libc::O_CLOEXEC) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { (OwnedFd::from_raw_fd(pipe[0]), OwnedFd::from_raw_fd(pipe[1])) })
}

#[cfg(target_os = "linux")]
fn attach_preexec_cgroup(
    directory: &OwnedFd,
    expected_parent: libc::pid_t,
    acknowledge: &OwnedFd,
) -> io::Result<()> {
    loop {
        if unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL, 0, 0, 0) } == 0 {
            break;
        }
        let error = raw_preexec_error();
        if error.raw_os_error() != Some(libc::EINTR) {
            return Err(error);
        }
    }
    if unsafe { libc::getppid() } != expected_parent {
        return Err(io::Error::from_raw_os_error(libc::ECHILD));
    }
    let name = b"cgroup.procs\0";
    let descriptor = loop {
        let descriptor = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                name.as_ptr().cast(),
                libc::O_WRONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if descriptor != -1 {
            break descriptor;
        }
        let error = raw_preexec_error();
        if error.raw_os_error() != Some(libc::EINTR) {
            return Err(error);
        }
    };
    let attached = raw_preexec_write_all(descriptor, b"0\n");
    let _ = unsafe { libc::close(descriptor) };
    attached?;
    let pid = unsafe { libc::getpid() }.to_ne_bytes();
    raw_preexec_write_all(acknowledge.as_raw_fd(), &pid)
}

#[cfg(target_os = "linux")]
impl Drop for LinuxCgroup {
    fn drop(&mut self) {
        if unsafe { libc::getpid() } != self.creator_pid {
            return;
        }
        if self.remove_if_empty().unwrap_or(false) {
            return;
        }
        let _ = self.kill();
        let _ = self.remove_if_empty();
    }
}

#[cfg(target_os = "linux")]
fn stable_cgroup_root() -> io::Result<std::path::PathBuf> {
    let uid = unsafe { libc::geteuid() };
    let canonical = canonical_cgroup_root(uid)?;
    validate_cgroup_filesystem(&canonical)?;
    validate_cgroup_controls(&canonical)?;
    let current = current_cgroup_path()?;
    if !current.starts_with(&canonical) {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "current process is outside the configured cgroup delegation",
        ));
    }
    Ok(canonical)
}

#[cfg(target_os = "linux")]
fn canonical_cgroup_root(uid: libc::uid_t) -> io::Result<std::path::PathBuf> {
    let configured = std::env::var_os("QOL_PROCESS_CGROUP_ROOT");
    let path = configured.as_ref().map_or_else(
        || {
            std::path::PathBuf::from(format!(
                "/sys/fs/cgroup/user.slice/user-{uid}.slice/user@{uid}.service"
            ))
        },
        std::path::PathBuf::from,
    );
    if !path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "QOL_PROCESS_CGROUP_ROOT must be absolute",
        ));
    }
    let canonical = std::fs::canonicalize(&path)?;
    if canonical != path {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "process cgroup delegation must use its canonical path",
        ));
    }
    let metadata = std::fs::symlink_metadata(&canonical)?;
    if !metadata.file_type().is_dir() || metadata.uid() != uid {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "process cgroup delegation must be an owned directory",
        ));
    }
    Ok(canonical)
}

#[cfg(target_os = "linux")]
fn validate_cgroup_filesystem(path: &std::path::Path) -> io::Result<()> {
    let path_c = CString::new(path.as_os_str().as_encoded_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "cgroup path contains NUL"))?;
    let mut filesystem: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(path_c.as_ptr(), &mut filesystem) } == -1 {
        return Err(io::Error::last_os_error());
    }
    if filesystem.f_type != libc::CGROUP2_SUPER_MAGIC {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "process cgroup delegation is not on cgroup v2",
        ));
    }
    if unsafe { libc::access(path_c.as_ptr(), libc::W_OK) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn validate_cgroup_controls(path: &std::path::Path) -> io::Result<()> {
    for control in [
        "cgroup.controllers",
        "cgroup.events",
        "cgroup.kill",
        "cgroup.procs",
    ] {
        if !path.join(control).is_file() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!("process cgroup delegation has no {control}"),
            ));
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn current_cgroup_path() -> io::Result<std::path::PathBuf> {
    let content = std::fs::read_to_string("/proc/self/cgroup")?;
    let current = content
        .lines()
        .find_map(|line| line.strip_prefix("0::"))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Unsupported,
                "process is not attached to a unified cgroup v2 hierarchy",
            )
        })?;
    let current = std::path::Path::new("/sys/fs/cgroup").join(current.trim_start_matches('/'));
    std::fs::canonicalize(current)
}

#[cfg(target_os = "linux")]
fn owned_cgroup_parent(
    root: &std::path::Path,
    journal_root: &std::path::Path,
) -> io::Result<std::path::PathBuf> {
    let current = current_cgroup_path()?;
    if current == root {
        return Ok(current);
    }
    for ancestor in current
        .ancestors()
        .take_while(|path| path.starts_with(root))
    {
        let Some(nonce) = ancestor
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_prefix("qol-process-v1-"))
        else {
            continue;
        };
        let path = journal_root.join(format!("{nonce}.lock"));
        let content = match std::fs::read_to_string(&path) {
            Ok(content) => content,
            Err(_) => continue,
        };
        let record = match CgroupJournal::parse(&content) {
            Ok(record) => record,
            Err(_) => continue,
        };
        if record.path == ancestor && validate_journal_record_identity(root, &path, &record)? {
            return Ok(current);
        }
    }
    if path_has_owned_component(root, &current) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "current process is below an unverified qol process cgroup",
        ));
    }
    Ok(current)
}

#[cfg(target_os = "linux")]
fn path_has_owned_component(root: &std::path::Path, path: &std::path::Path) -> bool {
    path.strip_prefix(root).is_ok_and(|relative| {
        relative.components().any(|component| {
            matches!(
                component,
                std::path::Component::Normal(name)
                    if name.to_str().is_some_and(|name| name.starts_with("qol-process-v1-"))
            )
        })
    })
}

#[cfg(target_os = "linux")]
fn valid_owned_cgroup_path(root: &std::path::Path, path: &std::path::Path) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        return false;
    };
    let components = relative.components().collect::<Vec<_>>();
    let Some(std::path::Component::Normal(name)) = components.last() else {
        return false;
    };
    components
        .iter()
        .all(|component| matches!(component, std::path::Component::Normal(_)))
        && name
            .to_str()
            .is_some_and(|name| name.starts_with("qol-process-v1-"))
}

#[cfg(target_os = "linux")]
fn validate_journal_record_identity(
    root: &std::path::Path,
    journal_path: &std::path::Path,
    record: &CgroupJournal,
) -> io::Result<bool> {
    let Some(nonce) = journal_path.file_stem().and_then(|stem| stem.to_str()) else {
        return Ok(false);
    };
    let expected_name = format!("qol-process-v1-{nonce}");
    if record.boot_id != linux_boot_id()?
        || !valid_owned_cgroup_path(root, &record.path)
        || record.path.file_name().and_then(|name| name.to_str()) != Some(expected_name.as_str())
    {
        return Ok(false);
    }
    let metadata = match std::fs::symlink_metadata(&record.path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    if !metadata.file_type().is_dir() {
        return Ok(false);
    }
    let directory = match open_directory(&record.path) {
        Ok(directory) => directory,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    let (device, inode) = directory_identity(&directory)?;
    Ok(device == record.device && inode == record.inode)
}

#[cfg(target_os = "linux")]
fn cgroup_journal_root() -> io::Result<std::path::PathBuf> {
    let uid = unsafe { libc::geteuid() };
    let configured = std::env::var_os("QOL_PROCESS_CGROUP_JOURNAL_ROOT");
    let root = configured.as_ref().map_or_else(
        || std::path::PathBuf::from(format!("/run/user/{uid}/qol-process-cgroups")),
        std::path::PathBuf::from,
    );
    if !root.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "QOL_PROCESS_CGROUP_JOURNAL_ROOT must be absolute",
        ));
    }
    if configured.is_some() {
        validate_journal_parent(&root, uid)?;
    }
    if !root.exists() {
        if configured.is_some() {
            create_journal_leaf(&root)?;
        } else {
            let mut builder = std::fs::DirBuilder::new();
            builder.recursive(true).mode(0o700).create(&root)?;
        }
    }
    let canonical = std::fs::canonicalize(&root)?;
    if canonical != root {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "process cgroup journal must use its canonical path",
        ));
    }
    let metadata = std::fs::symlink_metadata(&root)?;
    if !metadata.file_type().is_dir()
        || metadata.uid() != uid
        || metadata.permissions().mode() & 0o777 != 0o700
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "qol process cgroup journal root must be an owner-only 0700 directory",
        ));
    }
    Ok(root)
}

#[cfg(target_os = "linux")]
fn validate_journal_parent(root: &std::path::Path, uid: libc::uid_t) -> io::Result<()> {
    let parent = root.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "process cgroup journal override has no parent",
        )
    })?;
    let canonical_parent = std::fs::canonicalize(parent)?;
    if canonical_parent != parent {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "process cgroup journal parent must use its canonical path",
        ));
    }
    let metadata = std::fs::symlink_metadata(parent)?;
    if !metadata.file_type().is_dir()
        || metadata.uid() != uid
        || metadata.permissions().mode() & 0o022 != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "process cgroup journal parent must be an owner-controlled directory",
        ));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn create_journal_leaf(root: &std::path::Path) -> io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    match builder.mode(0o700).create(root) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(target_os = "linux")]
fn journal_namespace(root: &std::path::Path) -> io::Result<String> {
    let directory = open_directory(root)?;
    let (device, inode) = directory_identity(&directory)?;
    Ok(format!("{device:x}-{inode:x}"))
}

#[cfg(target_os = "linux")]
fn recover_stale_cgroups(
    root: &std::path::Path,
    journal_root: &std::path::Path,
    budget: &mut CgroupRecoveryBudget,
) -> io::Result<()> {
    loop {
        if !budget.can_start_pass() {
            return Ok(());
        }
        let cleaned = recover_stale_cgroup_pass(root, journal_root, budget)?;
        if !cleaned {
            return Ok(());
        }
    }
}

#[cfg(target_os = "linux")]
fn recover_stale_cgroup_pass(
    root: &std::path::Path,
    journal_root: &std::path::Path,
    budget: &mut CgroupRecoveryBudget,
) -> io::Result<bool> {
    let mut cleaned = false;
    for entry in std::fs::read_dir(journal_root)? {
        if !budget.claim_work() {
            return Ok(cleaned);
        }
        let entry = entry?;
        let entry_path = entry.path();
        if entry_path
            .extension()
            .and_then(|extension| extension.to_str())
            != Some("lock")
        {
            continue;
        }
        if !budget.claim_record() {
            return Ok(cleaned);
        }
        match load_recoverable_cgroup(root, &entry_path)? {
            RecoverableCgroupState::Pending => {}
            RecoverableCgroupState::Cleaned => cleaned = true,
            RecoverableCgroupState::Ready(cgroup) => {
                cleaned |= recover_stale_cgroup(cgroup, budget)?;
            }
        }
    }
    Ok(remove_empty_orphaned_cgroups(root, journal_root, budget)? || cleaned)
}

#[cfg(target_os = "linux")]
fn load_recoverable_cgroup(
    root: &std::path::Path,
    journal_path: &std::path::Path,
) -> io::Result<RecoverableCgroupState> {
    let Some(mut journal) = open_stale_journal(journal_path)? else {
        return Ok(RecoverableCgroupState::Pending);
    };
    let Some(record) = read_stale_cgroup_record(root, journal_path, &mut journal)? else {
        return Ok(RecoverableCgroupState::Pending);
    };
    open_recoverable_cgroup(journal_path, journal, record)
}

#[cfg(target_os = "linux")]
fn open_stale_journal(path: &std::path::Path) -> io::Result<Option<File>> {
    let journal = match OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(path)
    {
        Ok(journal) => journal,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !try_lock_journal(&journal)? {
        return Ok(None);
    }
    Ok(Some(journal))
}

#[cfg(target_os = "linux")]
fn read_stale_cgroup_record(
    root: &std::path::Path,
    journal_path: &std::path::Path,
    journal: &mut File,
) -> io::Result<Option<CgroupJournal>> {
    let mut content = String::new();
    journal.read_to_string(&mut content)?;
    let Ok(record) = CgroupJournal::parse(&content) else {
        quarantine_journal(journal_path)?;
        return Ok(None);
    };
    let Some(nonce) = journal_path.file_stem().and_then(|stem| stem.to_str()) else {
        quarantine_journal(journal_path)?;
        return Ok(None);
    };
    let expected_name = format!("qol-process-v1-{nonce}");
    let valid = record.boot_id == linux_boot_id()?
        && valid_owned_cgroup_path(root, &record.path)
        && record.path.file_name().and_then(|name| name.to_str()) == Some(expected_name.as_str());
    if valid {
        return Ok(Some(record));
    }
    quarantine_journal(journal_path)?;
    Ok(None)
}

#[cfg(target_os = "linux")]
fn open_recoverable_cgroup(
    journal_path: &std::path::Path,
    journal: File,
    record: CgroupJournal,
) -> io::Result<RecoverableCgroupState> {
    match std::fs::symlink_metadata(&record.path) {
        Ok(metadata) if metadata.file_type().is_dir() => {}
        Ok(_) => {
            quarantine_journal(journal_path)?;
            return Ok(RecoverableCgroupState::Pending);
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            remove_file_if_present(journal_path)?;
            return Ok(RecoverableCgroupState::Cleaned);
        }
        Err(_) => return Ok(RecoverableCgroupState::Pending),
    }
    let directory = match open_directory(&record.path) {
        Ok(directory) => directory,
        Err(_) => return Ok(RecoverableCgroupState::Pending),
    };
    if directory_identity(&directory)? != (record.device, record.inode) {
        quarantine_journal(journal_path)?;
        return Ok(RecoverableCgroupState::Pending);
    }
    Ok(RecoverableCgroupState::Ready(RecoverableCgroup {
        _journal: journal,
        journal_path: journal_path.to_path_buf(),
        record,
        directory,
    }))
}

#[cfg(target_os = "linux")]
fn recover_stale_cgroup(
    cgroup: RecoverableCgroup,
    budget: &mut CgroupRecoveryBudget,
) -> io::Result<bool> {
    if write_at(cgroup.directory.as_raw_fd(), "cgroup.kill", b"1").is_err()
        || !wait_for_recovered_cgroup_empty(&cgroup.directory, budget)?
        || !remove_descendant_cgroups_for_recovery(&cgroup.record.path, budget)?
        || !budget.claim_work()
    {
        return Ok(false);
    }
    verify_open_directory_path(&cgroup.directory, &cgroup.record.path)?;
    if std::fs::remove_dir(&cgroup.record.path).is_err() {
        return Ok(false);
    }
    std::fs::remove_file(&cgroup.journal_path)?;
    Ok(true)
}

#[cfg(target_os = "linux")]
fn wait_for_recovered_cgroup_empty(
    directory: &OwnedFd,
    budget: &CgroupRecoveryBudget,
) -> io::Result<bool> {
    loop {
        if !cgroup_populated(directory.as_raw_fd())? {
            return Ok(true);
        }
        let Some(remaining) = budget.remaining() else {
            return Ok(false);
        };
        std::thread::sleep(WAIT_INTERVAL.min(remaining));
    }
}

#[cfg(target_os = "linux")]
fn remove_file_if_present(path: &std::path::Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(target_os = "linux")]
fn remove_empty_orphaned_cgroups(
    root: &std::path::Path,
    journal_root: &std::path::Path,
    budget: &mut CgroupRecoveryBudget,
) -> io::Result<bool> {
    let Some(represented) = represented_cgroup_paths(root, journal_root, budget)? else {
        return Ok(false);
    };
    let owned_prefix = format!("qol-process-v1-{}-", journal_namespace(journal_root)?);
    let Some(mut candidates) = descendant_cgroups_for_recovery(root, budget)? else {
        return Ok(false);
    };
    candidates.sort_unstable_by_key(|entry| std::cmp::Reverse(entry.1));
    let mut cleaned = false;
    for (path, _) in candidates {
        if !budget.claim_work() {
            return Ok(cleaned);
        }
        if !is_unrepresented_owned_cgroup(&path, &owned_prefix, &represented) {
            continue;
        }
        cleaned |= remove_empty_orphaned_cgroup(&path, budget)?;
    }
    Ok(cleaned)
}

#[cfg(target_os = "linux")]
fn is_unrepresented_owned_cgroup(
    path: &std::path::Path,
    owned_prefix: &str,
    represented: &BTreeSet<std::path::PathBuf>,
) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with(owned_prefix))
        && !represented.contains(path)
        && !represented
            .iter()
            .any(|recorded| recorded.starts_with(path))
}

#[cfg(target_os = "linux")]
fn remove_empty_orphaned_cgroup(
    path: &std::path::Path,
    budget: &mut CgroupRecoveryBudget,
) -> io::Result<bool> {
    let directory = match open_directory(path) {
        Ok(directory) => directory,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    if cgroup_populated(directory.as_raw_fd())?
        || !remove_descendant_cgroups_for_recovery(path, budget)?
        || cgroup_populated(directory.as_raw_fd())?
    {
        return Ok(false);
    }
    verify_open_directory_path(&directory, path)?;
    match std::fs::remove_dir(path) {
        Ok(()) => Ok(true),
        Err(error) if retryable_cgroup_removal(&error) => Ok(false),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

#[cfg(target_os = "linux")]
fn represented_cgroup_paths(
    root: &std::path::Path,
    journal_root: &std::path::Path,
    budget: &mut CgroupRecoveryBudget,
) -> io::Result<Option<BTreeSet<std::path::PathBuf>>> {
    let mut represented = BTreeSet::new();
    for entry in std::fs::read_dir(journal_root)? {
        if !budget.claim_work() {
            return Ok(None);
        }
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("lock") {
            continue;
        }
        if !budget.claim_record() {
            return Ok(None);
        }
        let content = match std::fs::read_to_string(&path) {
            Ok(content) => content,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        let record = match CgroupJournal::parse(&content) {
            Ok(record) => record,
            Err(_) => continue,
        };
        if validate_journal_record_identity(root, &path, &record)? {
            represented.insert(record.path);
        }
    }
    Ok(Some(represented))
}

#[cfg(target_os = "linux")]
fn cgroup_populated(directory: libc::c_int) -> io::Result<bool> {
    read_at(directory, "cgroup.events")?
        .lines()
        .find_map(|line| line.strip_prefix("populated "))
        .map(|value| match value {
            "0" => Ok(false),
            "1" => Ok(true),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid cgroup populated value `{value}`"),
            )),
        })
        .unwrap_or_else(|| {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "cgroup.events has no populated state",
            ))
        })
}

#[cfg(target_os = "linux")]
fn quarantine_journal(path: &std::path::Path) -> io::Result<()> {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown.lock");
    let quarantine = path.with_file_name(format!("{file_name}.quarantine-{timestamp}"));
    std::fs::rename(path, quarantine)
}

#[cfg(target_os = "linux")]
fn linux_boot_id() -> io::Result<String> {
    let boot_id = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")?
        .trim()
        .to_string();
    if boot_id.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Linux boot id is empty",
        ));
    }
    Ok(boot_id)
}

#[cfg(target_os = "linux")]
fn lock_cgroup_registry(root: &std::path::Path, budget: &CgroupRecoveryBudget) -> io::Result<File> {
    let path = root.join("registry.guard");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(&path)?;
    loop {
        let Some(remaining) = budget.remaining() else {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "process cgroup registry lock {} remained busy until the recovery deadline",
                    path.display()
                ),
            ));
        };
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Ok(file);
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::EINTR) {
            continue;
        }
        if error.raw_os_error() != Some(libc::EWOULDBLOCK)
            && error.raw_os_error() != Some(libc::EAGAIN)
        {
            return Err(error);
        }
        std::thread::sleep(WAIT_INTERVAL.min(remaining));
    }
}

#[cfg(target_os = "linux")]
fn lock_journal(file: &File) -> io::Result<()> {
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        return Ok(());
    }
    Err(io::Error::last_os_error())
}

#[cfg(target_os = "linux")]
fn try_lock_journal(file: &File) -> io::Result<bool> {
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        return Ok(true);
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::EWOULDBLOCK) || error.raw_os_error() == Some(libc::EAGAIN)
    {
        return Ok(false);
    }
    Err(error)
}

#[cfg(target_os = "linux")]
fn open_directory(path: &std::path::Path) -> io::Result<OwnedFd> {
    let path = CString::new(path.as_os_str().as_encoded_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "cgroup path contains NUL"))?;
    let descriptor = unsafe {
        libc::open(
            path.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if descriptor == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(descriptor) })
}

#[cfg(target_os = "linux")]
fn open_at(directory: libc::c_int, name: &str, flags: libc::c_int) -> io::Result<OwnedFd> {
    let name = CString::new(name)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "control name contains NUL"))?;
    let descriptor = unsafe {
        libc::openat(
            directory,
            name.as_ptr(),
            flags | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if descriptor == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(descriptor) })
}

#[cfg(target_os = "linux")]
fn read_at(directory: libc::c_int, name: &str) -> io::Result<String> {
    let descriptor = open_at(directory, name, libc::O_RDONLY)?;
    let mut file = File::from(descriptor);
    let mut content = String::new();
    file.read_to_string(&mut content)?;
    Ok(content)
}

#[cfg(target_os = "linux")]
fn write_at(directory: libc::c_int, name: &str, content: &[u8]) -> io::Result<()> {
    let descriptor = open_at(directory, name, libc::O_WRONLY)?;
    let mut file = File::from(descriptor);
    file.write_all(content)
}

#[cfg(target_os = "linux")]
fn parse_cgroup_processes(content: &str) -> io::Result<Vec<libc::pid_t>> {
    content
        .lines()
        .map(|line| {
            line.parse::<libc::pid_t>().map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("invalid cgroup process id `{line}`: {error}"),
                )
            })
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn collect_descendant_members(
    root: &std::path::Path,
    members: &mut Vec<libc::pid_t>,
) -> io::Result<()> {
    for (path, _) in descendant_cgroups(root)? {
        let directory = match open_directory(&path) {
            Ok(directory) => directory,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        let processes = match read_at(directory.as_raw_fd(), "cgroup.procs") {
            Ok(processes) => processes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        members.extend(parse_cgroup_processes(&processes)?);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn remove_descendant_cgroups(root: &std::path::Path) -> io::Result<bool> {
    let mut descendants = descendant_cgroups(root)?;
    descendants.sort_unstable_by_key(|entry| std::cmp::Reverse(entry.1));
    for (path, _) in descendants {
        match std::fs::remove_dir(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) if retryable_cgroup_removal(&error) => return Ok(false),
            Err(error) => return Err(error),
        }
    }
    Ok(true)
}

#[cfg(target_os = "linux")]
fn remove_descendant_cgroups_for_recovery(
    root: &std::path::Path,
    budget: &mut CgroupRecoveryBudget,
) -> io::Result<bool> {
    let Some(mut descendants) = descendant_cgroups_for_recovery(root, budget)? else {
        return Ok(false);
    };
    descendants.sort_unstable_by_key(|entry| std::cmp::Reverse(entry.1));
    for (path, _) in descendants {
        if !budget.claim_work() {
            return Ok(false);
        }
        match std::fs::remove_dir(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) if retryable_cgroup_removal(&error) => return Ok(false),
            Err(error) => return Err(error),
        }
    }
    Ok(true)
}

#[cfg(target_os = "linux")]
fn descendant_cgroups_for_recovery(
    root: &std::path::Path,
    budget: &mut CgroupRecoveryBudget,
) -> io::Result<Option<Vec<(std::path::PathBuf, usize)>>> {
    let mut pending = vec![(root.to_path_buf(), 0_usize)];
    let mut descendants = Vec::new();
    while let Some((parent, depth)) = pending.pop() {
        let entries = match std::fs::read_dir(parent) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        for entry in entries {
            if !budget.claim_work() {
                return Ok(None);
            }
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            if !file_type.is_dir() {
                continue;
            }
            let Some(child_depth) = depth.checked_add(1) else {
                return Ok(None);
            };
            if child_depth > MAX_CGROUP_DEPTH || descendants.len() >= MAX_CGROUP_NODES {
                return Ok(None);
            }
            let path = entry.path();
            descendants.push((path.clone(), child_depth));
            pending.push((path, child_depth));
        }
    }
    Ok(Some(descendants))
}

#[cfg(target_os = "linux")]
fn descendant_cgroups(root: &std::path::Path) -> io::Result<Vec<(std::path::PathBuf, usize)>> {
    let mut pending = vec![(root.to_path_buf(), 0_usize)];
    let mut descendants = Vec::new();
    while let Some((parent, depth)) = pending.pop() {
        let entries = match std::fs::read_dir(parent) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            if !file_type.is_dir() {
                continue;
            }
            let child_depth = depth.checked_add(1).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "cgroup depth overflow")
            })?;
            if child_depth > MAX_CGROUP_DEPTH || descendants.len() >= MAX_CGROUP_NODES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "owned cgroup hierarchy exceeds the cleanup work bound",
                ));
            }
            let path = entry.path();
            descendants.push((path.clone(), child_depth));
            pending.push((path, child_depth));
        }
    }
    Ok(descendants)
}

#[cfg(target_os = "linux")]
fn retryable_cgroup_removal(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::DirectoryNotEmpty | io::ErrorKind::WouldBlock
    ) || matches!(
        error.raw_os_error(),
        Some(code) if code == libc::EBUSY || code == libc::ENOTEMPTY
    )
}

#[cfg(target_os = "linux")]
fn verify_open_directory_path(directory: &OwnedFd, path: &std::path::Path) -> io::Result<()> {
    let reopened = open_directory(path)?;
    if directory_identity(directory)? != directory_identity(&reopened)? {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "owned cgroup path was replaced before cleanup proof",
        ));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn directory_identity(directory: &OwnedFd) -> io::Result<(u64, u64)> {
    let mut metadata: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(directory.as_raw_fd(), &mut metadata) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok((metadata.st_dev, metadata.st_ino))
}

#[cfg(target_os = "linux")]
fn process_in_scope(pid: libc::pid_t, scope: ProcessScope) -> io::Result<bool> {
    let ProcessScope::Process(expected) = scope;
    Ok(pid == expected)
}

#[cfg(not(target_os = "linux"))]
fn process_in_scope(pid: libc::pid_t, scope: ProcessScope) -> io::Result<bool> {
    let actual = match scope {
        ProcessScope::Process(expected) => return Ok(pid == expected),
        ProcessScope::ProcessGroup(_) => unsafe { libc::getpgid(pid) },
        ProcessScope::Session(_) => unsafe { libc::getsid(pid) },
    };
    if actual == -1 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            return Ok(false);
        }
        return Err(error);
    }
    Ok(match scope {
        ProcessScope::Process(_) => unreachable!(),
        ProcessScope::ProcessGroup(expected) | ProcessScope::Session(expected) => {
            actual == expected
        }
    })
}

#[cfg(target_os = "macos")]
fn list_process_ids() -> io::Result<Vec<libc::pid_t>> {
    let count = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    if count < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut capacity = usize::try_from(count)
        .unwrap_or_default()
        .saturating_add(64)
        .max(64);
    loop {
        let mut pids = vec![0; capacity];
        let bytes = i32::try_from(capacity.saturating_mul(std::mem::size_of::<libc::pid_t>()))
            .map_err(|_| io::Error::other("process list buffer is too large"))?;
        let read = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), bytes) };
        if read < 0 {
            return Err(io::Error::last_os_error());
        }
        let read = usize::try_from(read).unwrap_or_default();
        if read >= capacity {
            capacity = capacity
                .checked_mul(2)
                .ok_or_else(|| io::Error::other("process list capacity overflow"))?;
            continue;
        }
        pids.truncate(read);
        pids.retain(|pid| *pid > 0);
        pids.sort_unstable();
        return Ok(pids);
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn list_process_ids() -> io::Result<Vec<libc::pid_t>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "verified process-scope enumeration is unsupported on this Unix platform",
    ))
}

pub(crate) fn own_current_process_tree_with_guardian(
    guardian_command: Command,
) -> io::Result<ProcessTreeGuard> {
    #[cfg(target_os = "linux")]
    {
        process_tree_containment_support()?;
        let cgroup = LinuxCgroup::create()?;
        let (kill, events) = cgroup.guardian_controls()?;
        let guardian = linux_guardian::Guardian::spawn(guardian_command, kill, events)?;
        Ok(ProcessTreeGuard {
            target: Mutex::new(None),
            guardian: Some(guardian),
            cgroup,
            prepared: AtomicBool::new(false),
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = guardian_command;
        Err(unsupported_process_tree_containment())
    }
}

pub(crate) fn run_process_tree_guardian_entry() -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        linux_guardian::run_entry()
    }
    #[cfg(not(target_os = "linux"))]
    Err(unsupported_process_tree_containment())
}

pub(crate) fn process_tree_containment_support() -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        let _ = stable_cgroup_root()?;
        let _ = cgroup_journal_root()?;
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    Err(unsupported_process_tree_containment())
}

pub(crate) fn isolate_owned_command(command: &mut Command) -> io::Result<()> {
    command.process_group(0);
    Ok(())
}

pub(crate) fn isolate_owned_session(command: &mut Command) -> io::Result<()> {
    unsafe {
        command.pre_exec(|| loop {
            if libc::setsid() != -1 {
                return Ok(());
            }
            let error = raw_preexec_error();
            if error.raw_os_error() != Some(libc::EINTR) {
                return Err(error);
            }
        });
    }
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
    let process_directory = open_directory(std::path::Path::new(&format!("/proc/{pid}")))?;
    let (device, inode) = directory_identity(&process_directory)?;
    let stat = read_at(process_directory.as_raw_fd(), "stat")?;
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
    Ok(format!(
        "linux:{}:{device}:{inode}:{start_ticks}",
        linux_boot_id()?
    ))
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

fn owned_signal_target(pid: libc::pid_t) -> libc::pid_t {
    if unsafe { libc::getpgid(pid) } == pid {
        return -pid;
    }
    pid
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn reused_root_identity_is_never_signaled() {
        let mut command = Command::new("sleep");
        command.arg("30");
        isolate_owned_session(&mut command).unwrap();
        let mut child = command.spawn().unwrap();
        let pid = pid_t(child.id()).unwrap();
        let mut root = capture_owned_process(pid).unwrap().unwrap();
        root.identity = "stale-process-identity".to_string();
        let error = signal_owned_process_handle(&root, libc::SIGTERM)
            .expect_err("stale ownership evidence must fail closed");

        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(is_pid_alive(child.id()));
        terminate_owned(&mut child, Duration::from_millis(20)).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn pidfd_generation_mismatch_is_never_signaled() {
        let mut first = Command::new("sleep").arg("30").spawn().unwrap();
        let mut second = Command::new("sleep").arg("30").spawn().unwrap();
        let first_pid = pid_t(first.id()).unwrap();
        let second_pid = pid_t(second.id()).unwrap();
        let first_process = capture_owned_process(first_pid).unwrap().unwrap();
        let second_process = capture_owned_process(second_pid).unwrap().unwrap();
        let mismatched = OwnedProcess {
            pid: second_process.pid,
            identity: second_process.identity,
            generation: second_process.generation,
            handle: first_process.handle,
        };

        let error = signal_owned_process_handle(&mismatched, libc::SIGTERM)
            .expect_err("a pidfd from another process generation must fail closed");

        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(is_pid_alive(first.id()));
        assert!(is_pid_alive(second.id()));
        terminate_owned(&mut first, Duration::from_millis(20)).unwrap();
        terminate_owned(&mut second, Duration::from_millis(20)).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn recovery_removes_an_empty_unjournaled_owned_cgroup() {
        let root = stable_cgroup_root().unwrap();
        let journal_root = cgroup_journal_root().unwrap();
        let budget = CgroupRecoveryBudget::standard().unwrap();
        let registry = lock_cgroup_registry(&journal_root, &budget).unwrap();
        let namespace = journal_namespace(&journal_root).unwrap();
        let nonce = format!(
            "{namespace}-{}-{}-0",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let orphan = root.join(format!("qol-process-v1-{nonce}"));
        std::fs::create_dir(&orphan).unwrap();
        drop(registry);

        let replacement = LinuxCgroup::create().unwrap();

        assert!(!orphan.exists());
        drop(replacement);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn registry_lock_contention_ends_at_the_shared_recovery_deadline() {
        let root = tempfile::tempdir().unwrap();
        let holder_budget =
            CgroupRecoveryBudget::new(Instant::now() + Duration::from_secs(1), 1, 1);
        let holder = lock_cgroup_registry(root.path(), &holder_budget).unwrap();
        let started = Instant::now();
        let waiter_budget = CgroupRecoveryBudget::new(started + Duration::from_millis(80), 1, 1);

        let error = lock_cgroup_registry(root.path(), &waiter_budget).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(error.to_string().contains("registry.guard"));
        assert!(started.elapsed() < Duration::from_secs(1));
        drop(holder);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn cgroup_walk_rejects_hierarchies_beyond_the_depth_bound() {
        let temp = tempfile::tempdir().unwrap();
        let mut path = temp.path().to_path_buf();
        for depth in 0..=MAX_CGROUP_DEPTH {
            path = path.join(depth.to_string());
            std::fs::create_dir(&path).unwrap();
        }

        let error = descendant_cgroups(temp.path()).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn stale_recovery_retains_records_beyond_its_global_budget() {
        let root = tempfile::tempdir().unwrap();
        let journals = tempfile::tempdir().unwrap();
        for index in 0..3 {
            std::fs::write(journals.path().join(format!("{index}.lock")), "invalid").unwrap();
        }
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut budget = CgroupRecoveryBudget::new(deadline, 1, 64);

        recover_stale_cgroup_pass(root.path(), journals.path(), &mut budget).unwrap();

        let pending = std::fs::read_dir(journals.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry.path().extension().and_then(|value| value.to_str()) == Some("lock")
            })
            .count();
        assert_eq!(pending, 2);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn stale_recovery_leaves_evidence_when_its_deadline_has_elapsed() {
        let root = tempfile::tempdir().unwrap();
        let journals = tempfile::tempdir().unwrap();
        let pending = journals.path().join("pending.lock");
        std::fs::write(&pending, "invalid").unwrap();
        let mut budget = CgroupRecoveryBudget::new(Instant::now(), 1, 64);

        let cleaned = recover_stale_cgroup_pass(root.path(), journals.path(), &mut budget).unwrap();

        assert!(!cleaned);
        assert!(pending.exists());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn stale_recovery_leaves_evidence_when_its_work_cap_is_exhausted() {
        let root = tempfile::tempdir().unwrap();
        let journals = tempfile::tempdir().unwrap();
        let pending = journals.path().join("pending.lock");
        std::fs::write(&pending, "invalid").unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut budget = CgroupRecoveryBudget::new(deadline, 1, 1);

        let cleaned = recover_stale_cgroup_pass(root.path(), journals.path(), &mut budget).unwrap();

        assert!(!cleaned);
        assert!(pending.exists());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_process_identity_includes_proc_directory_generation() {
        let identity = process_identity(std::process::id()).unwrap();
        let fields = identity.split(':').collect::<Vec<_>>();

        assert_eq!(fields.len(), 5);
        assert_eq!(fields[0], "linux");
        assert!(fields[2].parse::<u64>().is_ok());
        assert!(fields[3].parse::<u64>().is_ok());
        assert!(fields[4].parse::<u64>().is_ok());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn owned_cgroup_paths_allow_safe_arbitrary_intermediate_descendants() {
        let root = std::path::Path::new("/delegated");
        let nested = root
            .join("qol-process-v1-outer")
            .join("arbitrary-child")
            .join("qol-process-v1-inner");

        assert!(valid_owned_cgroup_path(root, &nested));
        assert!(!valid_owned_cgroup_path(
            root,
            &root.join("arbitrary-child")
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn stale_journal_recovery_kills_and_removes_the_exact_recorded_cgroup() {
        let stale = std::mem::ManuallyDrop::new(LinuxCgroup::create().unwrap());
        let stale_path = stale.path.clone();
        let stale_journal = stale.journal_path.clone();
        let mut command = Command::new("sleep");
        command.arg("30");
        let prepared = stale.prepare_command(&mut command).unwrap();
        let mut child = command.spawn().unwrap();
        assert_eq!(
            prepared.acknowledged_pid().unwrap(),
            pid_t(child.id()).unwrap()
        );
        let pid = child.id();
        let journal_lock = unsafe { std::ptr::read(&stale._journal_lock) };
        drop(journal_lock);

        let replacement = LinuxCgroup::create().unwrap();
        let status = child.wait().unwrap();

        assert!(!status.success());
        assert!(!is_pid_alive(pid));
        assert!(!stale_path.exists());
        assert!(!stale_journal.exists());
        drop(replacement);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn exact_process_signaling_fails_closed_without_an_identity_bound_handle() {
        let mut child = Command::new("sleep").arg("30").spawn().unwrap();
        let pid = pid_t(child.id()).unwrap();
        let process = capture_owned_process(pid).unwrap().unwrap();

        let error = signal_owned_process_handle(&process, libc::SIGTERM)
            .expect_err("raw PID signaling must not follow an identity check on macOS");

        assert_eq!(error.kind(), io::ErrorKind::Unsupported);
        assert!(is_pid_alive(child.id()));
        terminate_owned(&mut child, Duration::from_millis(20)).unwrap();
    }
}
