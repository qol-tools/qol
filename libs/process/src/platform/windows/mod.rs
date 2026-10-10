use std::ffi::OsString;
use std::io;
use std::os::windows::ffi::OsStringExt;
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::{CommandExt, ExitStatusExt};
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use qol_platform::native::wide::wide_nul;

use crate::{PlatformSpawnFailure, PreparedSpawnCleanup, ProcessEntry};

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, SetLastError, BOOL, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS,
    ERROR_FILE_NOT_FOUND, ERROR_INVALID_PARAMETER, ERROR_MORE_DATA, ERROR_NO_MORE_FILES, FILETIME,
    HANDLE, INVALID_HANDLE_VALUE, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::System::Console::{
    SetConsoleCtrlHandler, CTRL_BREAK_EVENT, CTRL_CLOSE_EVENT, CTRL_C_EVENT, CTRL_LOGOFF_EVENT,
    CTRL_SHUTDOWN_EVENT,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, Thread32First, Thread32Next,
    PROCESSENTRY32W, TH32CS_SNAPPROCESS, TH32CS_SNAPTHREAD, THREADENTRY32,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, IsProcessInJob,
    JobObjectBasicAccountingInformation, JobObjectBasicProcessIdList,
    JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
    TerminateJobObject, JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_BASIC_PROCESS_ID_LIST,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK,
};
use windows_sys::Win32::System::Threading::{
    CreateEventW, GetCurrentProcess, GetExitCodeProcess, GetProcessTimes, OpenEventW, OpenProcess,
    QueryFullProcessImageNameW, ResetEvent, SetEvent, TerminateProcess, WaitForSingleObject,
    CREATE_SUSPENDED, EVENT_MODIFY_STATE, INFINITE, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_QUOTA, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
    THREAD_SUSPEND_RESUME,
};
use windows_sys::Win32::System::Threading::{OpenThread, ResumeThread};

const QUERY_AND_WAIT_ACCESS: u32 = PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE;
const TERMINATE_AND_WAIT_ACCESS: u32 =
    PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE;
const WAIT_INTERVAL: Duration = Duration::from_millis(50);
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const KILL_SETTLE: Duration = Duration::from_secs(1);
const STOP_EVENT_PREFIX: &str = "Local\\qol-stop-";
const FILETIME_UNIX_EPOCH: u64 = 116_444_736_000_000_000;
const IMAGE_PATH_CAPACITY: usize = 32_768;
static CANCELLATION_SIGNAL_COUNT: AtomicUsize = AtomicUsize::new(0);
static CANCELLATION_INSTALL: OnceLock<Result<(), i32>> = OnceLock::new();
static STOP_LISTENER_INSTALL: OnceLock<Result<(), i32>> = OnceLock::new();
static STOP_SIGNAL: Mutex<()> = Mutex::new(());
static STOP_SIGNALLED: Condvar = Condvar::new();

struct JobHandle(HANDLE);

unsafe impl Send for JobHandle {}

impl Drop for JobHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

struct ThreadHandle(HANDLE);

impl Drop for ThreadHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

struct SnapshotHandle(HANDLE);

impl Drop for SnapshotHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

pub(crate) struct ProcessTreeGuard {
    job: JobHandle,
    assigned_process: Mutex<Option<AssignedProcess>>,
    terminating_processes: Mutex<Vec<ProcessHandle>>,
    prepared: AtomicBool,
}

pub(crate) struct PreparedSpawn {
    failed_child_reaper: FailedChildReaper,
}

struct FailedChildReaper {
    state: Arc<(Mutex<FailedChildReaperState>, Condvar)>,
}

struct FailedChildReaperState {
    child: Option<Child>,
    closed: bool,
}

struct AssignedProcess {
    id: u32,
    handle: ProcessHandle,
}

pub(crate) struct CurrentProcessTreeGuard {
    job: Option<JobHandle>,
    armed: bool,
}

impl Drop for CurrentProcessTreeGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        if let Some(job) = self.job.take() {
            std::mem::forget(job);
        }
    }
}

impl ProcessTreeGuard {
    pub(crate) fn containment_backend(&self) -> &'static str {
        "windows_job_object"
    }

    pub(crate) fn membership_observation_supported(&self) -> bool {
        false
    }

    pub(crate) fn observe_residual(&self) -> crate::ProcessTreeObservation {
        crate::ProcessTreeObservation::unsupported()
    }

    pub(crate) fn prepare_command(&self, command: &mut Command) -> io::Result<PreparedSpawn> {
        if self.prepared.swap(true, Ordering::AcqRel) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "process tree already prepared a command",
            ));
        }
        let failed_child_reaper = FailedChildReaper::start()?;
        command.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_SUSPENDED);
        Ok(PreparedSpawn {
            failed_child_reaper,
        })
    }

    pub(crate) fn spawn_prepared(
        &self,
        command: &mut Command,
        prepared: PreparedSpawn,
    ) -> Result<Child, PlatformSpawnFailure> {
        let child = match command.spawn() {
            Ok(child) => child,
            Err(source) => {
                return Err(PlatformSpawnFailure {
                    source,
                    cleanup: PreparedSpawnCleanup::NotStarted,
                });
            }
        };
        if let Err(error) = self.assign_and_resume(&child) {
            let cleanup = self.abort_suspended_child(child, prepared.failed_child_reaper);
            return Err(prepared_spawn_failure(error, cleanup));
        }
        Ok(child)
    }

    pub(crate) fn abort_prepared(&self) {}

    fn assign_and_resume(&self, child: &Child) -> io::Result<()> {
        let mut assigned_process = self
            .assigned_process
            .lock()
            .map_err(|_| io::Error::other("process-tree assignment state is unavailable"))?;
        if assigned_process.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "process tree already owns a process",
            ));
        }
        let owned_process = AssignedProcess {
            id: child.id(),
            handle: open_process(child.id(), TERMINATE_AND_WAIT_ACCESS)?,
        };
        *assigned_process = Some(owned_process);
        let process = child.as_raw_handle() as HANDLE;
        if unsafe { AssignProcessToJobObject(self.job.0, process) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let thread = sole_process_thread(child.id())?;
        let previous = unsafe { ResumeThread(thread.0) };
        if previous == u32::MAX {
            return Err(io::Error::last_os_error());
        }
        if previous != 1 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("prepared process primary thread had suspend count {previous}, expected 1"),
            ));
        }
        Ok(())
    }

    fn abort_suspended_child(
        &self,
        mut child: Child,
        failed_child_reaper: FailedChildReaper,
    ) -> io::Result<()> {
        let _ = unsafe { TerminateJobObject(self.job.0, 1) };
        let _ = unsafe { TerminateProcess(child.as_raw_handle() as HANDLE, 1) };
        let wait = unsafe {
            WaitForSingleObject(
                child.as_raw_handle() as HANDLE,
                duration_millis(Duration::from_secs(2)),
            )
        };
        match wait {
            WAIT_OBJECT_0 => match child.wait() {
                Ok(_) => Ok(()),
                Err(error) => {
                    failed_child_reaper.handoff(child);
                    Err(error)
                }
            },
            WAIT_TIMEOUT => {
                failed_child_reaper.handoff(child);
                Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "failed prepared process did not terminate while suspended",
                ))
            }
            WAIT_FAILED => {
                let error = io::Error::last_os_error();
                failed_child_reaper.handoff(child);
                Err(error)
            }
            other => {
                failed_child_reaper.handoff(child);
                Err(io::Error::other(format!(
                    "unexpected failed-process wait result {other}"
                )))
            }
        }
    }

    pub(crate) fn terminate_and_wait(&self, timeout: Duration) -> io::Result<()> {
        let process_id = self.assigned_process_id()?;
        let deadline = Instant::now().checked_add(timeout).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "process-tree timeout is too large",
            )
        })?;
        let processes = self.termination_processes()?;
        if unsafe { TerminateJobObject(self.job.0, 1) } == 0 {
            return Err(io::Error::last_os_error());
        }
        wait_for_process_handles(&processes, deadline, timeout).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("process tree rooted at {process_id} did not exit: {error}"),
            )
        })?;
        wait_for_job_empty(self.job.0, deadline, timeout)
    }

    pub(crate) fn request_stop(&self) -> io::Result<()> {
        self.assigned_process_id()?;
        let processes = job_process_handles(self.job.0)?;
        self.terminating_processes
            .lock()
            .map_err(|_| io::Error::other("process-tree termination state is unavailable"))?
            .extend(processes);
        if unsafe { TerminateJobObject(self.job.0, 1) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub(crate) fn force_stop_and_wait(&self, timeout: Duration) -> io::Result<()> {
        self.terminate_and_wait(timeout)
    }

    pub(crate) fn recover_pending_spawn(&self, timeout: Duration) -> io::Result<()> {
        let assigned_process = self
            .assigned_process
            .lock()
            .map_err(|_| io::Error::other("process-tree assignment state is unavailable"))?;
        let process = assigned_process.as_ref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotConnected,
                "pending process tree has no exact process handle",
            )
        })?;
        let deadline = Instant::now().checked_add(timeout).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "process-tree timeout is too large",
            )
        })?;
        let processes = job_process_handles(self.job.0)?;
        let _ = unsafe { TerminateJobObject(self.job.0, 1) };
        if unsafe { WaitForSingleObject(process.handle.0, 0) } == WAIT_TIMEOUT {
            let _ = unsafe { TerminateProcess(process.handle.0, 1) };
        }
        wait_for_process_handle(process, deadline, timeout)?;
        wait_for_process_handles(&processes, deadline, timeout)?;
        wait_for_job_empty(self.job.0, deadline, timeout)
    }

    pub(crate) fn terminate_root_and_wait(&self, timeout: Duration) -> io::Result<()> {
        let assigned_process = self
            .assigned_process
            .lock()
            .map_err(|_| io::Error::other("process-tree assignment state is unavailable"))?;
        let process = assigned_process.as_ref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotConnected,
                "process tree has no assigned process",
            )
        })?;
        let wait = unsafe { WaitForSingleObject(process.handle.0, 0) };
        if wait == WAIT_OBJECT_0 {
            return Ok(());
        }
        if wait == WAIT_FAILED {
            return Err(io::Error::last_os_error());
        }
        if unsafe { TerminateProcess(process.handle.0, 1) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let wait = unsafe { WaitForSingleObject(process.handle.0, duration_millis(timeout)) };
        match wait {
            WAIT_OBJECT_0 => Ok(()),
            WAIT_TIMEOUT => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "owned process root PID {} did not exit within {timeout:?}",
                    process.id
                ),
            )),
            WAIT_FAILED => Err(io::Error::last_os_error()),
            other => Err(io::Error::other(format!(
                "unexpected process wait result {other}"
            ))),
        }
    }

    pub(crate) fn root_has_exited(&self) -> io::Result<bool> {
        let assigned_process = self
            .assigned_process
            .lock()
            .map_err(|_| io::Error::other("process-tree assignment state is unavailable"))?;
        let process = assigned_process.as_ref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotConnected,
                "process tree has no assigned process",
            )
        })?;
        match unsafe { WaitForSingleObject(process.handle.0, 0) } {
            WAIT_OBJECT_0 => Ok(true),
            WAIT_TIMEOUT => Ok(false),
            WAIT_FAILED => Err(io::Error::last_os_error()),
            other => Err(io::Error::other(format!(
                "unexpected process wait result {other}"
            ))),
        }
    }

    pub(crate) fn tree_has_exited(&self) -> io::Result<bool> {
        active_processes(self.job.0).map(|count| count == 0)
    }

    fn assigned_process_id(&self) -> io::Result<u32> {
        self.assigned_process
            .lock()
            .map_err(|_| io::Error::other("process-tree assignment state is unavailable"))?
            .as_ref()
            .map(|process| process.id)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotConnected,
                    "process tree has no assigned process",
                )
            })
    }

    fn termination_processes(&self) -> io::Result<Vec<ProcessHandle>> {
        let current = job_process_handles(self.job.0)?;
        let mut processes = self
            .terminating_processes
            .lock()
            .map_err(|_| io::Error::other("process-tree termination state is unavailable"))?;
        let mut captured = std::mem::take(&mut *processes);
        captured.extend(current);
        Ok(captured)
    }
}

fn wait_for_process_handle(
    process: &AssignedProcess,
    deadline: Instant,
    timeout: Duration,
) -> io::Result<()> {
    let wait = unsafe {
        WaitForSingleObject(
            process.handle.0,
            duration_millis(deadline.saturating_duration_since(Instant::now())),
        )
    };
    match wait {
        WAIT_OBJECT_0 => Ok(()),
        WAIT_TIMEOUT => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!(
                "pending process {} did not exit within {timeout:?}",
                process.id
            ),
        )),
        WAIT_FAILED => Err(io::Error::last_os_error()),
        other => Err(io::Error::other(format!(
            "unexpected pending-process wait result {other}"
        ))),
    }
}

fn wait_for_process_handles(
    processes: &[ProcessHandle],
    deadline: Instant,
    timeout: Duration,
) -> io::Result<()> {
    for process in processes {
        let wait = unsafe {
            WaitForSingleObject(
                process.0,
                duration_millis(deadline.saturating_duration_since(Instant::now())),
            )
        };
        match wait {
            WAIT_OBJECT_0 => {}
            WAIT_TIMEOUT => {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("owned process did not exit within {timeout:?}"),
                ));
            }
            WAIT_FAILED => return Err(io::Error::last_os_error()),
            other => {
                return Err(io::Error::other(format!(
                    "unexpected owned-process wait result {other}"
                )));
            }
        }
    }
    Ok(())
}

fn wait_for_job_empty(job: HANDLE, deadline: Instant, timeout: Duration) -> io::Result<()> {
    loop {
        if active_processes(job)? == 0 {
            return Ok(());
        }
        let now = Instant::now();
        if now >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("pending process job did not empty within {timeout:?}"),
            ));
        }
        std::thread::sleep(WAIT_INTERVAL.min(deadline.duration_since(now)));
    }
}

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

impl Drop for FailedChildReaper {
    fn drop(&mut self) {
        let (state, ready) = &*self.state;
        let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
        state.closed = true;
        ready.notify_one();
    }
}

fn sole_process_thread(process_id: u32) -> io::Result<ThreadHandle> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let snapshot = SnapshotHandle(snapshot);
    let mut entry: THREADENTRY32 = unsafe { std::mem::zeroed() };
    entry.dwSize = u32::try_from(std::mem::size_of::<THREADENTRY32>())
        .map_err(|_| io::Error::other("thread entry is too large"))?;
    if unsafe { Thread32First(snapshot.0, &mut entry) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut thread_id = None;
    loop {
        if entry.th32OwnerProcessID == process_id && thread_id.replace(entry.th32ThreadID).is_some()
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "suspended prepared process unexpectedly has multiple threads",
            ));
        }
        if unsafe { Thread32Next(snapshot.0, &mut entry) } != 0 {
            continue;
        }
        let error = unsafe { GetLastError() };
        if error != ERROR_NO_MORE_FILES {
            return Err(io::Error::from_raw_os_error(error as i32));
        }
        break;
    }
    let thread_id = thread_id.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "suspended prepared process has no discoverable primary thread",
        )
    })?;
    let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, thread_id) };
    if thread.is_null() {
        return Err(io::Error::last_os_error());
    }
    Ok(ThreadHandle(thread))
}

fn prepared_spawn_failure(error: io::Error, cleanup: io::Result<()>) -> PlatformSpawnFailure {
    match cleanup {
        Ok(()) => PlatformSpawnFailure {
            source: error,
            cleanup: PreparedSpawnCleanup::Verified,
        },
        Err(cleanup) => PlatformSpawnFailure {
            source: io::Error::new(
                error.kind(),
                format!("{error}; failed to clean the suspended process: {cleanup}"),
            ),
            cleanup: PreparedSpawnCleanup::RecoveryPending,
        },
    }
}

impl CurrentProcessTreeGuard {
    pub(crate) fn disarm(&mut self) -> io::Result<()> {
        if !self.armed {
            return Ok(());
        }
        let job = self
            .job
            .as_ref()
            .ok_or_else(|| io::Error::other("current process-tree job is unavailable"))?;
        configure_kill_on_close(job.0, false)?;
        self.armed = false;
        Ok(())
    }
}

pub(crate) fn own_current_process_tree_with_guardian(
    guardian_command: Command,
) -> io::Result<ProcessTreeGuard> {
    drop(guardian_command);
    own_process_tree()
}

fn own_process_tree() -> io::Result<ProcessTreeGuard> {
    process_tree_containment_support()?;
    Ok(ProcessTreeGuard {
        job: create_kill_on_close_job()?,
        assigned_process: Mutex::new(None),
        terminating_processes: Mutex::new(Vec::new()),
        prepared: AtomicBool::new(false),
    })
}

pub(crate) fn spawn_owned(mut command: Command) -> io::Result<(Child, Option<ProcessTreeGuard>)> {
    let guard = own_process_tree()?;
    let prepared = guard.prepare_command(&mut command)?;
    let child = guard
        .spawn_prepared(&mut command, prepared)
        .map_err(|error| prepared_spawn_error(&guard, error))?;
    Ok((child, Some(guard)))
}

fn prepared_spawn_error(guard: &ProcessTreeGuard, error: PlatformSpawnFailure) -> io::Error {
    let PlatformSpawnFailure { source, cleanup } = error;
    if cleanup != PreparedSpawnCleanup::RecoveryPending {
        return source;
    }
    match guard.recover_pending_spawn(Duration::from_secs(2)) {
        Ok(()) => source,
        Err(recovery) => io::Error::new(
            source.kind(),
            format!("{source}; process-tree recovery failed: {recovery}"),
        ),
    }
}

pub(crate) fn run_process_tree_guardian_entry() -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Windows process trees use kernel-owned Job Objects instead of a guardian process",
    ))
}

pub(crate) fn process_tree_containment_support() -> io::Result<()> {
    let _ = create_kill_on_close_job()?;
    Ok(())
}

pub(crate) fn isolate_owned_command(command: &mut Command) -> io::Result<()> {
    command.creation_flags(CREATE_NEW_PROCESS_GROUP);
    Ok(())
}

pub(crate) fn isolate_owned_session(command: &mut Command) -> io::Result<()> {
    isolate_owned_command(command)
}

pub(crate) fn guard_current_process_tree() -> io::Result<CurrentProcessTreeGuard> {
    let job = create_kill_on_close_job()?;
    if unsafe { AssignProcessToJobObject(job.0, GetCurrentProcess()) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(CurrentProcessTreeGuard {
        job: Some(job),
        armed: true,
    })
}

fn create_kill_on_close_job() -> io::Result<JobHandle> {
    let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    let job = JobHandle(handle);
    configure_kill_on_close(handle, true)?;
    Ok(job)
}

fn configure_kill_on_close(handle: HANDLE, enabled: bool) -> io::Result<()> {
    set_job_limit_flags(
        handle,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE * u32::from(enabled),
    )
}

static HOST_LIFETIME_JOB: Mutex<Option<usize>> = Mutex::new(None);

pub(crate) fn bind_to_host_lifetime(pid: u32) -> io::Result<()> {
    let job = host_lifetime_job()?;
    let process = open_process(pid, PROCESS_SET_QUOTA | PROCESS_TERMINATE)?;
    if unsafe { AssignProcessToJobObject(job, process.0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn host_lifetime_job() -> io::Result<HANDLE> {
    let mut slot = HOST_LIFETIME_JOB
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(job) = *slot {
        return Ok(job as HANDLE);
    }
    let job = create_kill_on_close_job()?;
    set_job_limit_flags(
        job.0,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK,
    )?;
    let handle = job.0;
    std::mem::forget(job);
    *slot = Some(handle as usize);
    Ok(handle)
}

fn set_job_limit_flags(handle: HANDLE, flags: u32) -> io::Result<()> {
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    limits.BasicLimitInformation.LimitFlags = flags;
    let configured = unsafe {
        SetInformationJobObject(
            handle,
            JobObjectExtendedLimitInformation,
            std::ptr::from_ref(&limits).cast(),
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    if configured == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn active_processes(handle: HANDLE) -> io::Result<u32> {
    let mut accounting: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { std::mem::zeroed() };
    let queried = unsafe {
        QueryInformationJobObject(
            handle,
            JobObjectBasicAccountingInformation,
            std::ptr::from_mut(&mut accounting).cast(),
            std::mem::size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
            std::ptr::null_mut(),
        )
    };
    if queried == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(accounting.ActiveProcesses)
}

fn job_process_handles(handle: HANDLE) -> io::Result<Vec<ProcessHandle>> {
    let mut processes = Vec::new();
    for pid in job_process_ids(handle)? {
        match open_process(pid, QUERY_AND_WAIT_ACCESS) {
            Ok(process) => processes.push(process),
            Err(error) if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(processes)
}

fn job_process_ids(handle: HANDLE) -> io::Result<Vec<u32>> {
    let mut capacity = 8usize;
    loop {
        let header = std::mem::offset_of!(JOBOBJECT_BASIC_PROCESS_ID_LIST, ProcessIdList);
        let bytes = header
            .checked_add(
                capacity
                    .checked_mul(std::mem::size_of::<usize>())
                    .ok_or_else(|| io::Error::other("process ID buffer is too large"))?,
            )
            .ok_or_else(|| io::Error::other("process ID buffer is too large"))?;
        let words = bytes.div_ceil(std::mem::size_of::<usize>());
        let mut buffer = vec![0usize; words];
        let buffer_bytes = u32::try_from(words * std::mem::size_of::<usize>())
            .map_err(|_| io::Error::other("process ID buffer is too large"))?;
        let queried = unsafe {
            QueryInformationJobObject(
                handle,
                JobObjectBasicProcessIdList,
                buffer.as_mut_ptr().cast(),
                buffer_bytes,
                std::ptr::null_mut(),
            )
        };
        if queried == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_MORE_DATA as i32) {
                capacity = capacity
                    .checked_mul(2)
                    .ok_or_else(|| io::Error::other("process ID buffer is too large"))?;
                continue;
            }
            return Err(error);
        }
        let info = buffer.as_ptr().cast::<JOBOBJECT_BASIC_PROCESS_ID_LIST>();
        let count = unsafe { (*info).NumberOfProcessIdsInList as usize };
        if count > capacity {
            return Err(io::Error::other(
                "job returned more process IDs than the query buffer holds",
            ));
        }
        let ids = unsafe {
            std::slice::from_raw_parts(
                std::ptr::addr_of!((*info).ProcessIdList).cast::<usize>(),
                count,
            )
        };
        return ids
            .iter()
            .copied()
            .map(|pid| u32::try_from(pid).map_err(|_| io::Error::other("process ID exceeds u32")))
            .collect();
    }
}

pub(crate) fn install_cancellation_handler() -> io::Result<()> {
    let console = CANCELLATION_INSTALL.get_or_init(|| {
        if unsafe { SetConsoleCtrlHandler(Some(cancellation_control_handler), 1) } != 0 {
            return Ok(());
        }
        Err(io::Error::last_os_error().raw_os_error().unwrap_or(1))
    });
    if let Err(code) = console {
        return Err(io::Error::from_raw_os_error(*code));
    }
    match STOP_LISTENER_INSTALL.get_or_init(|| start_stop_listener().map_err(raw_error_code)) {
        Ok(()) => Ok(()),
        Err(code) => Err(io::Error::from_raw_os_error(*code)),
    }
}

pub(crate) fn wait_for_stop_request() -> io::Result<()> {
    install_cancellation_handler()?;
    let mut signal = STOP_SIGNAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    while cancellation_signal_count() == 0 {
        signal = STOP_SIGNALLED
            .wait(signal)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
    }
    Ok(())
}

pub(crate) fn cancellation_requested() -> bool {
    cancellation_signal_count() > 0
}

pub(crate) fn cancellation_signal_count() -> usize {
    CANCELLATION_SIGNAL_COUNT.load(Ordering::Acquire)
}

fn record_cancellation_signal() {
    let _signal = STOP_SIGNAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    CANCELLATION_SIGNAL_COUNT.fetch_add(1, Ordering::Release);
    STOP_SIGNALLED.notify_all();
}

unsafe extern "system" fn cancellation_control_handler(control: u32) -> BOOL {
    if !matches!(
        control,
        CTRL_C_EVENT
            | CTRL_BREAK_EVENT
            | CTRL_CLOSE_EVENT
            | CTRL_LOGOFF_EVENT
            | CTRL_SHUTDOWN_EVENT
    ) {
        return 0;
    }
    record_cancellation_signal();
    1
}

struct EventHandle(HANDLE);

unsafe impl Send for EventHandle {}

impl EventHandle {
    fn wait(&self) -> u32 {
        unsafe { WaitForSingleObject(self.0, INFINITE) }
    }
}

impl Drop for EventHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

fn stop_event_name(pid: u32) -> Vec<u16> {
    wide_nul(format!("{STOP_EVENT_PREFIX}{pid}"))
}

fn start_stop_listener() -> io::Result<()> {
    let name = stop_event_name(std::process::id());
    unsafe { SetLastError(0) };
    let event = unsafe { CreateEventW(std::ptr::null(), 0, 0, name.as_ptr()) };
    if event.is_null() {
        return Err(io::Error::last_os_error());
    }
    let event = EventHandle(event);
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS && unsafe { ResetEvent(event.0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    std::thread::Builder::new()
        .name("qol-process-stop-listener".to_string())
        .spawn(move || loop {
            if event.wait() != WAIT_OBJECT_0 {
                return;
            }
            record_cancellation_signal();
        })?;
    Ok(())
}

fn request_graceful_stop(pid: u32) -> io::Result<bool> {
    let name = stop_event_name(pid);
    let event = unsafe { OpenEventW(EVENT_MODIFY_STATE, 0, name.as_ptr()) };
    if event.is_null() {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(ERROR_FILE_NOT_FOUND as i32) {
            return Ok(false);
        }
        return Err(error);
    }
    let event = EventHandle(event);
    if unsafe { SetEvent(event.0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(true)
}

fn raw_error_code(error: io::Error) -> i32 {
    error.raw_os_error().unwrap_or(1)
}

struct ProcessHandle(HANDLE);

unsafe impl Send for ProcessHandle {}

impl Drop for ProcessHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

pub(crate) fn is_pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    let handle = match open_process(pid, QUERY_AND_WAIT_ACCESS) {
        Ok(handle) => handle,
        Err(error) => {
            return error.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32);
        }
    };
    unsafe { WaitForSingleObject(handle.0, 0) == WAIT_TIMEOUT }
}

pub(crate) fn is_group_alive(pid: u32) -> bool {
    is_pid_alive(pid)
}

pub(crate) fn is_pid_zombie(_pid: u32) -> bool {
    false
}

pub(crate) fn process_identity(pid: u32) -> io::Result<String> {
    let process = open_process(pid, QUERY_AND_WAIT_ACCESS)?;
    if unsafe { WaitForSingleObject(process.0, 0) } != WAIT_TIMEOUT {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("process {pid} has exited"),
        ));
    }
    Ok(format!("windows:{}", process_creation_time(&process)?))
}

pub(crate) fn process_identity_matches(actual: &str, expected: &str) -> bool {
    actual == expected
}

fn process_creation_time(process: &ProcessHandle) -> io::Result<u64> {
    let mut creation: FILETIME = unsafe { std::mem::zeroed() };
    let mut exit: FILETIME = unsafe { std::mem::zeroed() };
    let mut kernel: FILETIME = unsafe { std::mem::zeroed() };
    let mut user: FILETIME = unsafe { std::mem::zeroed() };
    if unsafe { GetProcessTimes(process.0, &mut creation, &mut exit, &mut kernel, &mut user) } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok((u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime))
}

struct Member {
    pid: u32,
    created: u64,
    handle: ProcessHandle,
}

impl Member {
    fn open(pid: u32) -> io::Result<Self> {
        let handle = open_process(pid, TERMINATE_AND_WAIT_ACCESS)?;
        let created = process_creation_time(&handle)?;
        Ok(Self {
            pid,
            created,
            handle,
        })
    }

    fn running(&self) -> bool {
        unsafe { WaitForSingleObject(self.handle.0, 0) == WAIT_TIMEOUT }
    }

    fn in_job(&self) -> bool {
        let mut result: BOOL = 0;
        unsafe {
            IsProcessInJob(self.handle.0, std::ptr::null_mut(), &mut result) != 0 && result != 0
        }
    }

    fn request_stop_or_kill(&self) -> bool {
        if !self.running() {
            return false;
        }
        if request_graceful_stop(self.pid).unwrap_or(false) {
            return true;
        }
        self.kill();
        false
    }

    fn kill(&self) -> bool {
        unsafe { TerminateProcess(self.handle.0, 1) != 0 }
    }
}

fn filetime_now() -> u64 {
    let since_epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let ticks = u64::try_from(since_epoch.as_nanos() / 100).unwrap_or(u64::MAX);
    FILETIME_UNIX_EPOCH.saturating_add(ticks)
}

pub(crate) fn processes() -> io::Result<Vec<ProcessEntry>> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let snapshot = SnapshotHandle(snapshot);
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = u32::try_from(std::mem::size_of::<PROCESSENTRY32W>())
        .map_err(|_| io::Error::other("process entry is too large"))?;
    if unsafe { Process32FirstW(snapshot.0, &mut entry) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut entries = Vec::new();
    loop {
        let name = &entry.szExeFile;
        let end = name
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(name.len());
        entries.push(ProcessEntry {
            pid: entry.th32ProcessID,
            parent: entry.th32ParentProcessID,
            exe: String::from_utf16_lossy(&name[..end]),
        });
        if unsafe { Process32NextW(snapshot.0, &mut entry) } != 0 {
            continue;
        }
        let error = unsafe { GetLastError() };
        if error != ERROR_NO_MORE_FILES {
            return Err(io::Error::from_raw_os_error(error as i32));
        }
        return Ok(entries);
    }
}

pub(crate) fn process_image_path(pid: u32) -> io::Result<PathBuf> {
    let process = open_process(pid, PROCESS_QUERY_LIMITED_INFORMATION)?;
    let mut buffer = vec![0u16; IMAGE_PATH_CAPACITY];
    let mut length = u32::try_from(buffer.len()).unwrap_or(u32::MAX);
    let read = unsafe {
        QueryFullProcessImageNameW(
            process.0,
            PROCESS_NAME_WIN32,
            buffer.as_mut_ptr(),
            &mut length,
        )
    };
    if read == 0 {
        return Err(io::Error::last_os_error());
    }
    buffer.truncate(length as usize);
    Ok(PathBuf::from(OsString::from_wide(&buffer)))
}

pub(crate) fn hide_console_window(command: &mut Command) -> &mut Command {
    command.creation_flags(CREATE_NO_WINDOW)
}

fn extend_with_descendants(members: &mut Vec<Member>, contained: bool) -> io::Result<()> {
    let own_pid = std::process::id();
    loop {
        let observed_at = filetime_now();
        let mut added = false;
        for ProcessEntry { pid, parent, .. } in processes()? {
            if pid == 0 || pid == own_pid || members.iter().any(|member| member.pid == pid) {
                continue;
            }
            let Some(parent_created) = members
                .iter()
                .find(|member| member.pid == parent)
                .map(|member| member.created)
            else {
                continue;
            };
            let Ok(member) = Member::open(pid) else {
                continue;
            };
            if member.created < parent_created || member.created > observed_at {
                continue;
            }
            if contained && !member.in_job() {
                continue;
            }
            members.push(member);
            added = true;
        }
        if !added {
            return Ok(());
        }
    }
}

fn process_group(pid: u32) -> io::Result<Vec<Member>> {
    let root = Member::open(pid)?;
    let contained = root.in_job();
    let mut members = vec![root];
    extend_with_descendants(&mut members, contained)?;
    Ok(members)
}

fn wait_for_members(members: &[Member], deadline: Instant) -> bool {
    members.iter().all(|member| {
        let remaining = deadline.saturating_duration_since(Instant::now());
        unsafe { WaitForSingleObject(member.handle.0, duration_millis(remaining)) == WAIT_OBJECT_0 }
    })
}

fn stop_members(mut members: Vec<Member>, grace: Duration, include_descendants: bool) {
    for member in members.iter().filter(|member| member.running()) {
        let _ = request_graceful_stop(member.pid);
    }
    wait_for_members(&members, Instant::now() + grace);
    if include_descendants {
        let contained = members.first().is_some_and(Member::in_job);
        let _ = extend_with_descendants(&mut members, contained);
    }
    for member in members.iter().filter(|member| member.running()) {
        member.kill();
    }
    wait_for_members(&members, Instant::now() + KILL_SETTLE);
}

pub(crate) fn signal_term_pid(pid: u32) -> io::Result<()> {
    let member = Member::open(pid)?;
    if !member.running() || request_graceful_stop(pid)? {
        return Ok(());
    }
    if member.kill() {
        return Ok(());
    }
    Err(io::Error::last_os_error())
}

pub(crate) fn signal_term_group(pid: u32) -> io::Result<()> {
    for member in process_group(pid)? {
        member.request_stop_or_kill();
    }
    Ok(())
}

pub(crate) fn kill_pid(pid: u32) -> io::Result<()> {
    let handle = open_process(pid, TERMINATE_AND_WAIT_ACCESS)?;
    if unsafe { TerminateProcess(handle.0, 1) } != 0 {
        return Ok(());
    }
    Err(io::Error::last_os_error())
}

pub(crate) fn kill_group(pid: u32) -> io::Result<()> {
    let members = process_group(pid)?;
    let root_error = members
        .first()
        .and_then(|root| (!root.kill()).then(io::Error::last_os_error))
        .filter(|_| members.first().is_some_and(Member::running));
    for member in members.iter().skip(1) {
        member.kill();
    }
    root_error.map_or(Ok(()), Err)
}

pub(crate) fn try_wait_pid(pid: u32) -> io::Result<Option<ExitStatus>> {
    if pid == 0 {
        return Err(invalid_pid());
    }
    let handle = match open_process(pid, QUERY_AND_WAIT_ACCESS) {
        Ok(handle) => handle,
        Err(error) if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32) => {
            return Ok(Some(ExitStatus::from_raw(0)));
        }
        Err(error) => return Err(error),
    };
    let wait = unsafe { WaitForSingleObject(handle.0, 0) };
    if wait == WAIT_TIMEOUT {
        return Ok(None);
    }
    if wait == WAIT_FAILED {
        return Err(io::Error::last_os_error());
    }
    if wait != WAIT_OBJECT_0 {
        return Err(io::Error::other(format!(
            "unexpected process wait result {wait}"
        )));
    }
    let mut exit_code = 0;
    if unsafe { GetExitCodeProcess(handle.0, &mut exit_code) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(Some(ExitStatus::from_raw(exit_code)))
}

pub(crate) fn wait_pid(pid: u32) -> io::Result<ExitStatus> {
    loop {
        if let Some(status) = try_wait_pid(pid)? {
            return Ok(status);
        }
        std::thread::sleep(WAIT_INTERVAL);
    }
}

pub(crate) fn terminate_pid(pid: u32, grace: Duration) {
    let Ok(member) = Member::open(pid) else {
        return;
    };
    stop_members(vec![member], grace, false);
}

pub(crate) fn reload_group(pid: u32, grace: Duration) {
    terminate_group(pid, grace);
}

pub(crate) fn terminate_group(pid: u32, grace: Duration) {
    let Ok(members) = process_group(pid) else {
        return;
    };
    stop_members(members, grace, true);
}

pub(crate) fn terminate_owned(child: &mut Child, grace: Duration) -> io::Result<()> {
    if child.try_wait()?.is_some() {
        return Ok(());
    }
    if request_graceful_stop(child.id()).unwrap_or(false) {
        let deadline = Instant::now() + grace;
        while Instant::now() < deadline {
            if child.try_wait()?.is_some() {
                return Ok(());
            }
            std::thread::sleep(
                WAIT_INTERVAL.min(deadline.saturating_duration_since(Instant::now())),
            );
        }
    }
    child.kill()?;
    child.wait()?;
    Ok(())
}

pub(crate) fn spawn_detached(command: &mut Command) -> io::Result<()> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
    let child = command.spawn()?;
    drop(child);
    Ok(())
}

fn open_process(pid: u32, access: u32) -> io::Result<ProcessHandle> {
    if pid == 0 {
        return Err(invalid_pid());
    }
    let handle = unsafe { OpenProcess(access, 0, pid) };
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    Ok(ProcessHandle(handle))
}

fn duration_millis(duration: Duration) -> u32 {
    duration.as_millis().min(u32::MAX as u128) as u32
}

fn invalid_pid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "pid must be positive")
}
