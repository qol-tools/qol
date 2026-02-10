use std::time::Duration;

use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::System::Threading::{
    OpenProcess, TerminateProcess, WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_TERMINATE, SYNCHRONIZE, WAIT_TIMEOUT,
};

const QUERY_AND_WAIT_ACCESS: u32 = PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE;
const TERMINATE_AND_WAIT_ACCESS: u32 =
    PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE;

pub fn is_pid_alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }

    unsafe {
        let handle = OpenProcess(QUERY_AND_WAIT_ACCESS, 0, pid as u32);
        if handle == 0 {
            return false;
        }
        let wait_result = WaitForSingleObject(handle, 0);
        let _ = CloseHandle(handle);
        wait_result == WAIT_TIMEOUT
    }
}

pub fn terminate_pid(pid: i32, grace: Duration) {
    if pid <= 0 {
        return;
    }

    unsafe {
        let handle = OpenProcess(TERMINATE_AND_WAIT_ACCESS, 0, pid as u32);
        if handle == 0 {
            return;
        }

        let _ = TerminateProcess(handle, 1);
        let wait_ms = grace.as_millis().min(u32::MAX as u128) as u32;
        let _ = WaitForSingleObject(handle, wait_ms);
        let _ = CloseHandle(handle);
    }
}

pub fn reap_children_nonblocking() {}
