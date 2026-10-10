use std::sync::OnceLock;

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ACCESS_DENIED, FILETIME, HANDLE, WAIT_OBJECT_0,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetProcessTimes, OpenProcess, WaitForSingleObject,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
};

enum Parent {
    Watched(isize),
    Gone,
    Unknown,
}

static PARENT: OnceLock<Parent> = OnceLock::new();

pub(super) fn is_supported() -> bool {
    PARENT.get_or_init(open_parent);
    true
}

pub(super) fn is_orphaned() -> bool {
    match PARENT.get_or_init(open_parent) {
        Parent::Watched(handle) => unsafe {
            WaitForSingleObject(*handle as HANDLE, 0) == WAIT_OBJECT_0
        },
        Parent::Gone => true,
        Parent::Unknown => false,
    }
}

fn open_parent() -> Parent {
    let Some(parent_pid) = parent_pid() else {
        return Parent::Unknown;
    };
    let handle = unsafe {
        OpenProcess(
            PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
            0,
            parent_pid,
        )
    };
    if handle.is_null() {
        if unsafe { GetLastError() } == ERROR_ACCESS_DENIED {
            return Parent::Unknown;
        }
        return Parent::Gone;
    }
    let started_before_us = match (
        creation_time(handle),
        creation_time(unsafe { GetCurrentProcess() }),
    ) {
        (Some(parent), Some(own)) => parent <= own,
        _ => true,
    };
    if !started_before_us {
        unsafe { CloseHandle(handle) };
        return Parent::Gone;
    }
    Parent::Watched(handle as isize)
}

fn parent_pid() -> Option<u32> {
    let own_pid = std::process::id();
    qol_process::processes()
        .ok()?
        .into_iter()
        .find(|entry| entry.pid == own_pid)
        .map(|entry| entry.parent)
        .filter(|pid| *pid != 0)
}

fn creation_time(process: HANDLE) -> Option<u64> {
    let mut creation: FILETIME = unsafe { std::mem::zeroed() };
    let mut exit: FILETIME = unsafe { std::mem::zeroed() };
    let mut kernel: FILETIME = unsafe { std::mem::zeroed() };
    let mut user: FILETIME = unsafe { std::mem::zeroed() };
    let read =
        unsafe { GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user) };
    (read != 0)
        .then(|| (u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_live_parent_is_watched_and_not_orphaned() {
        assert!(parent_pid().is_some(), "the test runner has a parent");
        assert!(is_supported());
        assert!(!is_orphaned(), "the test runner's parent is still alive");
    }
}
