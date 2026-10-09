use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::Security::{
    GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};

pub(super) fn current() -> Option<bool> {
    token_elevated(unsafe { GetCurrentProcess() })
}

pub(super) fn of_process(pid: u32) -> Option<bool> {
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return None;
    }
    let elevated = token_elevated(process);
    unsafe { CloseHandle(process) };
    elevated
}

pub(super) fn can_inject(own: Option<bool>, target: Option<bool>) -> bool {
    own == Some(true) || target == Some(false)
}

fn token_elevated(process: HANDLE) -> Option<bool> {
    let mut token: HANDLE = std::ptr::null_mut();
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
        return None;
    }
    let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
    let mut length = 0u32;
    let read = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            (&mut elevation as *mut TOKEN_ELEVATION).cast(),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut length,
        )
    };
    unsafe { CloseHandle(token) };
    (read != 0).then_some(elevation.TokenIsElevated != 0)
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
