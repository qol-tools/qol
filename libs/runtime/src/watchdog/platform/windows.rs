use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::sync::OnceLock;

use windows_sys::Win32::Foundation::{GetLastError, ERROR_ACCESS_DENIED, WAIT_OBJECT_0};
use windows_sys::Win32::System::Threading::{
    OpenProcess, WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
};

enum Parent {
    Watched(OwnedHandle),
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
            WaitForSingleObject(handle.as_raw_handle(), 0) == WAIT_OBJECT_0
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
    let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
    let started_before_us = match (
        qol_process::process_start_time(parent_pid),
        qol_process::process_start_time(std::process::id()),
    ) {
        (Some(parent), Some(own)) => parent <= own,
        _ => true,
    };
    if !started_before_us {
        return Parent::Gone;
    }
    Parent::Watched(handle)
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
