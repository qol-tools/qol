use mach2::mach_time::{mach_timebase_info, mach_timebase_info_data_t};
use std::mem::MaybeUninit;
use std::sync::OnceLock;

pub(super) fn cpu_percent_window_samples() -> usize {
    5
}

pub(super) fn process_cpu_micros(pid: i32) -> Option<u64> {
    if pid <= 0 {
        return None;
    }

    let mut usage = MaybeUninit::<libc::rusage_info_v2>::uninit();
    let result =
        unsafe { libc::proc_pid_rusage(pid, libc::RUSAGE_INFO_V2, usage.as_mut_ptr() as _) };
    if result < 0 {
        return None;
    }

    let usage = unsafe { usage.assume_init() };
    let (numer, denom) = mach_timebase_ratio()?;
    let total_absolute = usage.ri_user_time as u128 + usage.ri_system_time as u128;
    let total_nanos = total_absolute.saturating_mul(numer as u128) / denom as u128;
    Some((total_nanos / 1_000) as u64)
}

fn mach_timebase_ratio() -> Option<(u64, u64)> {
    static TIMEBASE_RATIO: OnceLock<Option<(u64, u64)>> = OnceLock::new();
    *TIMEBASE_RATIO.get_or_init(|| {
        let mut info = mach_timebase_info_data_t { numer: 0, denom: 0 };
        let result = unsafe { mach_timebase_info(&mut info) };
        if result != 0 {
            return None;
        }
        if info.numer == 0 || info.denom == 0 {
            return None;
        }
        Some((info.numer as u64, info.denom as u64))
    })
}
