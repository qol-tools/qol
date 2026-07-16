use std::io;
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::{CommandExt, ExitStatusExt};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    CloseHandle, BOOL, ERROR_ACCESS_DENIED, ERROR_INVALID_PARAMETER, FILETIME, HANDLE, WAIT_FAILED,
    WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::System::Console::{
    SetConsoleCtrlHandler, CTRL_BREAK_EVENT, CTRL_CLOSE_EVENT, CTRL_C_EVENT, CTRL_LOGOFF_EVENT,
    CTRL_SHUTDOWN_EVENT,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicAccountingInformation,
    JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
    TerminateJobObject, JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetExitCodeProcess, GetProcessTimes, OpenProcess, TerminateProcess,
    WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
};

const QUERY_AND_WAIT_ACCESS: u32 = PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE;
const TERMINATE_AND_WAIT_ACCESS: u32 =
    PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE;
const WAIT_INTERVAL: Duration = Duration::from_millis(50);
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
static CANCELLATION_REQUESTED: AtomicBool = AtomicBool::new(false);
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

pub(crate) struct ProcessTreeGuard {
    job: JobHandle,
    assigned_process: Mutex<Option<u32>>,
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
    pub(crate) fn assign(&self, child: &Child) -> io::Result<()> {
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
        let process = child.as_raw_handle() as HANDLE;
        if unsafe { AssignProcessToJobObject(self.job.0, process) } != 0 {
            *assigned_process = Some(child.id());
            return Ok(());
        }
        Err(io::Error::last_os_error())
    }

    pub(crate) fn terminate_and_wait(&self, timeout: Duration) -> io::Result<()> {
        let assigned_process = self
            .assigned_process
            .lock()
            .map_err(|_| io::Error::other("process-tree assignment state is unavailable"))?;
        let process_id = (*assigned_process).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotConnected,
                "process tree has no assigned process",
            )
        })?;
        let deadline = Instant::now().checked_add(timeout).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "process-tree timeout is too large",
            )
        })?;
        if unsafe { TerminateJobObject(self.job.0, 1) } == 0 {
            return Err(io::Error::last_os_error());
        }
        loop {
            if active_processes(self.job.0)? == 0 {
                return Ok(());
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("process tree rooted at {process_id} did not exit within {timeout:?}"),
                ));
            }
            std::thread::sleep(WAIT_INTERVAL.min(deadline.duration_since(now)));
        }
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

pub(crate) fn own_current_process_tree() -> io::Result<ProcessTreeGuard> {
    Ok(ProcessTreeGuard {
        job: create_kill_on_close_job()?,
        assigned_process: Mutex::new(None),
    })
}

pub(crate) fn isolate_owned_command(command: &mut Command) -> io::Result<()> {
    command.creation_flags(CREATE_NEW_PROCESS_GROUP);
    Ok(())
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
    CANCELLATION_REQUESTED.load(Ordering::Acquire)
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
    CANCELLATION_REQUESTED.store(true, Ordering::Release);
    1
}

struct ProcessHandle(HANDLE);

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

pub(crate) fn signal_term_pid(pid: u32) -> io::Result<()> {
    kill_pid(pid)
}

pub(crate) fn kill_pid(pid: u32) -> io::Result<()> {
    let handle = open_process(pid, TERMINATE_AND_WAIT_ACCESS)?;
    if unsafe { TerminateProcess(handle.0, 1) } != 0 {
        return Ok(());
    }
    Err(io::Error::last_os_error())
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

pub(crate) fn reap_children_nonblocking() {}

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
