use std::io;
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::{CommandExt, ExitStatusExt};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::{PlatformSpawnFailure, PreparedSpawnCleanup};

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, BOOL, ERROR_ACCESS_DENIED, ERROR_INVALID_PARAMETER, ERROR_MORE_DATA,
    ERROR_NO_MORE_FILES, FILETIME, HANDLE, INVALID_HANDLE_VALUE, WAIT_FAILED, WAIT_OBJECT_0,
    WAIT_TIMEOUT,
};
use windows_sys::Win32::System::Console::{
    SetConsoleCtrlHandler, CTRL_BREAK_EVENT, CTRL_CLOSE_EVENT, CTRL_C_EVENT, CTRL_LOGOFF_EVENT,
    CTRL_SHUTDOWN_EVENT,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicAccountingInformation,
    JobObjectBasicProcessIdList, JobObjectExtendedLimitInformation, QueryInformationJobObject,
    SetInformationJobObject, TerminateJobObject, JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
    JOBOBJECT_BASIC_PROCESS_ID_LIST, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetExitCodeProcess, GetProcessTimes, OpenProcess, TerminateProcess,
    WaitForSingleObject, CREATE_SUSPENDED, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
    PROCESS_TERMINATE, THREAD_SUSPEND_RESUME,
};
use windows_sys::Win32::System::Threading::{OpenThread, ResumeThread};

const QUERY_AND_WAIT_ACCESS: u32 = PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE;
const TERMINATE_AND_WAIT_ACCESS: u32 =
    PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE;
const WAIT_INTERVAL: Duration = Duration::from_millis(50);
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
static CANCELLATION_SIGNAL_COUNT: AtomicUsize = AtomicUsize::new(0);
static CANCELLATION_INSTALL: OnceLock<Result<(), i32>> = OnceLock::new();

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
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    limits.BasicLimitInformation.LimitFlags =
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE * u32::from(enabled);
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
    let result = CANCELLATION_INSTALL.get_or_init(|| {
        if unsafe { SetConsoleCtrlHandler(Some(cancellation_control_handler), 1) } != 0 {
            return Ok(());
        }
        Err(io::Error::last_os_error().raw_os_error().unwrap_or(1))
    });
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
    CANCELLATION_SIGNAL_COUNT.fetch_add(1, Ordering::Release);
    1
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
    let mut creation: FILETIME = unsafe { std::mem::zeroed() };
    let mut exit: FILETIME = unsafe { std::mem::zeroed() };
    let mut kernel: FILETIME = unsafe { std::mem::zeroed() };
    let mut user: FILETIME = unsafe { std::mem::zeroed() };
    if unsafe { GetProcessTimes(process.0, &mut creation, &mut exit, &mut kernel, &mut user) } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let created = (u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime);
    Ok(format!("windows:{created}"))
}

pub(crate) fn process_identity_matches(actual: &str, expected: &str) -> bool {
    actual == expected
}

pub(crate) fn signal_term_pid(pid: u32) -> io::Result<()> {
    kill_pid(pid)
}

pub(crate) fn signal_term_group(_pid: u32) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Windows process groups do not provide verified tree termination",
    ))
}

pub(crate) fn kill_pid(pid: u32) -> io::Result<()> {
    let handle = open_process(pid, TERMINATE_AND_WAIT_ACCESS)?;
    if unsafe { TerminateProcess(handle.0, 1) } != 0 {
        return Ok(());
    }
    Err(io::Error::last_os_error())
}

pub(crate) fn kill_group(_pid: u32) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Windows process groups do not provide verified tree termination",
    ))
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
    let Ok(handle) = open_process(pid, TERMINATE_AND_WAIT_ACCESS) else {
        return;
    };
    unsafe {
        let _ = TerminateProcess(handle.0, 1);
        let _ = WaitForSingleObject(handle.0, duration_millis(grace));
    }
}

pub(crate) fn terminate_group(pid: u32, grace: Duration) {
    terminate_pid(pid, grace);
}

pub(crate) fn terminate_owned(child: &mut Child, _: Duration) -> io::Result<()> {
    if child.try_wait()?.is_some() {
        return Ok(());
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
