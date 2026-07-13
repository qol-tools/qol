use std::io;
use std::os::windows::process::{CommandExt, ExitStatusExt};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::Duration;

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, ERROR_INVALID_PARAMETER, HANDLE, WAIT_FAILED, WAIT_OBJECT_0,
    WAIT_TIMEOUT,
};
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, OpenProcess, TerminateProcess, WaitForSingleObject,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
};

const QUERY_AND_WAIT_ACCESS: u32 = PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE;
const TERMINATE_AND_WAIT_ACCESS: u32 =
    PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE;
const WAIT_INTERVAL: Duration = Duration::from_millis(50);
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

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
