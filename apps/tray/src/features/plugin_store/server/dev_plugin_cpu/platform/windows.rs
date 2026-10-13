use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};

use windows_sys::Win32::Foundation::FILETIME;
use windows_sys::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};

use super::DevPluginCpuPlatformOps;

pub(super) struct Platform;

impl DevPluginCpuPlatformOps for Platform {
    fn cpu_percent_window_samples() -> usize {
        1
    }

    fn process_cpu_micros(pid: i32) -> Option<u64> {
        let pid = u32::try_from(pid).ok().filter(|pid| *pid > 0)?;
        let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if raw.is_null() {
            return None;
        }
        let process = unsafe { OwnedHandle::from_raw_handle(raw) };
        let zero = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
        let ok = unsafe {
            GetProcessTimes(
                process.as_raw_handle(),
                &mut created,
                &mut exited,
                &mut kernel,
                &mut user,
            )
        };
        if ok == 0 {
            return None;
        }
        Some((hundred_nanoseconds(kernel) + hundred_nanoseconds(user)) / 10)
    }
}

fn hundred_nanoseconds(time: FILETIME) -> u64 {
    (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_current_process_reports_its_cpu_time_and_invalid_pids_report_none() {
        let pid = i32::try_from(std::process::id()).unwrap();
        let before = Platform::process_cpu_micros(pid).unwrap();
        let advanced = (0..1_000).any(|_| {
            let mut spin = 0u64;
            for value in 0..1_000_000u64 {
                spin = spin.wrapping_add(std::hint::black_box(value));
            }
            std::hint::black_box(spin);
            Platform::process_cpu_micros(pid).unwrap() > before
        });
        assert!(advanced);
        assert_eq!(Platform::process_cpu_micros(0), None);
        assert_eq!(Platform::process_cpu_micros(-1), None);
    }
}
