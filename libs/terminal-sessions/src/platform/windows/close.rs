use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, WAIT_OBJECT_0};
use windows_sys::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, TerminateProcess, WaitForSingleObject,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
};

const UNIX_EPOCH_AS_FILETIME_US: u64 = 11_644_473_600_000_000;
const FORCED_EXIT: u32 = 1;

pub(super) struct Member {
    handle: HANDLE,
    start: u64,
}

impl Drop for Member {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.handle) };
    }
}

impl Member {
    pub(super) fn open(pid: i32) -> Option<Member> {
        let pid = u32::try_from(pid).ok().filter(|pid| *pid != 0)?;
        let access = PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE | PROCESS_TERMINATE;
        let handle = unsafe { OpenProcess(access, 0, pid) };
        if handle.is_null() {
            return None;
        }
        let mut member = Member { handle, start: 0 };
        member.start = member.read_start()?;
        Some(member)
    }

    fn read_start(&self) -> Option<u64> {
        let empty = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let [mut created, mut exited, mut kernel, mut user] = [empty; 4];
        let read = unsafe {
            GetProcessTimes(
                self.handle,
                &mut created,
                &mut exited,
                &mut kernel,
                &mut user,
            )
        };
        if read == 0 {
            return None;
        }
        let since_1601_us =
            ((u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime)) / 10;
        since_1601_us.checked_sub(UNIX_EPOCH_AS_FILETIME_US)
    }

    pub(super) fn start(&self) -> u64 {
        self.start
    }

    pub(super) fn terminate(&self, code: u32) -> bool {
        unsafe { TerminateProcess(self.handle, code) != 0 }
    }

    fn exited_by(&self, deadline: Instant) -> bool {
        let wait = deadline.saturating_duration_since(Instant::now());
        let millis = u32::try_from(wait.as_millis()).unwrap_or(u32::MAX);
        unsafe { WaitForSingleObject(self.handle, millis) == WAIT_OBJECT_0 }
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
        let own = i32::try_from(std::process::id()).unwrap();
        let member = Member::open(own).expect("own process must open");
        assert_eq!(
            Some(member.start()),
            qol_app_icon::process_start_time_us(own)
        );
        assert!(Member::open(0).is_none());
    }
}
