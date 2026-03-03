use std::mem::MaybeUninit;

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
    let total_nanos = usage.ri_user_time as u128 + usage.ri_system_time as u128;
    Some((total_nanos / 1_000) as u64)
}
