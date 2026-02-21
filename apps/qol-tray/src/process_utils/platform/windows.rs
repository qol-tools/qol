use std::time::Duration;

use windows_sys::Win32::Foundation::{CloseHandle, WAIT_TIMEOUT};
use windows_sys::Win32::System::Threading::{
    OpenProcess, TerminateProcess, WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
};

const QUERY_AND_WAIT_ACCESS: u32 = PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE;
const TERMINATE_AND_WAIT_ACCESS: u32 =
    PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE;

pub(super) fn is_pid_alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }

    // SAFETY: OpenProcess/WaitForSingleObject/CloseHandle are called with validated handles.
    unsafe {
        let handle = OpenProcess(QUERY_AND_WAIT_ACCESS, 0, pid as u32);
        if handle.is_null() {
            return false;
        }
        let wait_result = WaitForSingleObject(handle, 0);
        let _ = CloseHandle(handle);
        wait_result == WAIT_TIMEOUT
    }
}

pub(super) fn terminate_pid(pid: i32, grace: Duration) {
    if pid <= 0 {
        return;
    }

    // SAFETY: Uses process handle APIs with null-handle checks and always closes handle.
    unsafe {
        let handle = OpenProcess(TERMINATE_AND_WAIT_ACCESS, 0, pid as u32);
        if handle.is_null() {
            return;
        }

        let _ = TerminateProcess(handle, 1);
        let wait_ms = grace.as_millis().min(u32::MAX as u128) as u32;
        let _ = WaitForSingleObject(handle, wait_ms);
        let _ = CloseHandle(handle);
    }
}

pub(super) fn reap_children_nonblocking() {}
