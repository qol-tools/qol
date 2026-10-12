use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};

use qol_platform::native::security::token_elevated;
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};

pub(super) fn current() -> Option<bool> {
    token_elevated(unsafe { GetCurrentProcess() })
}

pub(super) fn of_process(pid: u32) -> Option<bool> {
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return None;
    }
    let process = unsafe { OwnedHandle::from_raw_handle(process) };
    token_elevated(process.as_raw_handle())
}

pub(super) fn can_inject(own: Option<bool>, target: Option<bool>) -> bool {
    own == Some(true) || target == Some(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn injection_needs_an_elevated_self_or_a_known_unelevated_target() {
        let cases = [
            (Some(true), Some(true), true),
            (Some(true), None, true),
            (Some(false), Some(false), true),
            (None, Some(false), true),
            (Some(false), Some(true), false),
            (Some(false), None, false),
            (None, None, false),
        ];
        for (own, target, expected) in cases {
            assert_eq!(can_inject(own, target), expected, "{own:?} {target:?}");
        }
    }

    #[test]
    fn this_process_reports_its_elevation() {
        assert!(current().is_some());
        assert_eq!(of_process(std::process::id()), current());
    }
}
