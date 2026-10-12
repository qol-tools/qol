use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
use windows_sys::Win32::System::Threading::{
    OpenProcess, TerminateProcess, WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
};

const FORCED_EXIT: u32 = 1;

pub(super) struct Member {
    handle: OwnedHandle,
    start: u64,
}

impl Member {
    pub(super) fn open(pid: i32) -> Option<Member> {
        let pid = u32::try_from(pid).ok().filter(|pid| *pid != 0)?;
        let access = PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE | PROCESS_TERMINATE;
        let handle = unsafe { OpenProcess(access, 0, pid) };
        if handle.is_null() {
            return None;
        }
        let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
        let start = qol_process::process_start_time(pid)?;
        Some(Member { handle, start })
    }

    pub(super) fn start(&self) -> u64 {
        self.start
    }

    pub(super) fn terminate(&self, code: u32) -> bool {
        unsafe { TerminateProcess(self.handle.as_raw_handle(), code) != 0 }
    }

    fn exited_by(&self, deadline: Instant) -> bool {
        let wait = deadline.saturating_duration_since(Instant::now());
        let millis = u32::try_from(wait.as_millis()).unwrap_or(u32::MAX);
        unsafe { WaitForSingleObject(self.handle.as_raw_handle(), millis) == WAIT_OBJECT_0 }
    }
}

pub(super) fn token_start(native: &str) -> Option<u64> {
    native.split_once('-')?.1.parse().ok()
}

pub(super) fn belongs(root_start: u64, is_root: bool, start: u64) -> bool {
    if is_root {
        start == root_start
    } else {
        start >= root_start
    }
}

pub(super) fn settle(members: &[Member], grace: Duration) {
    let deadline = Instant::now() + grace;
    for member in members {
        if !member.exited_by(deadline) {
            member.terminate(FORCED_EXIT);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_start_reads_the_start_time_after_the_pid() {
        let cases = [
            ("4120-1700000000123456", Some(1_700_000_000_123_456)),
            ("4120", None),
            ("4120-", None),
            ("4120-abc", None),
        ];
        for (native, expected) in cases {
            assert_eq!(token_start(native), expected, "{native}");
        }
    }

    #[test]
    fn only_the_same_root_and_later_members_belong_to_the_session() {
        let cases = [
            (true, 100, true),
            (true, 101, false),
            (true, 99, false),
            (false, 100, true),
            (false, 250, true),
            (false, 99, false),
        ];
        for (is_root, start, expected) in cases {
            assert_eq!(belongs(100, is_root, start), expected, "{is_root} {start}");
        }
    }

    #[test]
    fn the_current_process_opens_with_its_own_start_time() {
        let own = std::process::id();
        let member = Member::open(i32::try_from(own).unwrap()).expect("own process must open");
        assert_eq!(Some(member.start()), qol_process::process_start_time(own));
        assert!(Member::open(0).is_none());
    }
}
