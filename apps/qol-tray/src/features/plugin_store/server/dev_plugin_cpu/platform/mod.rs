#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

pub(super) fn process_cpu_micros(pid: i32) -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        return linux::process_cpu_micros(pid);
    }

    #[cfg(target_os = "macos")]
    {
        return macos::process_cpu_micros(pid);
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = pid;
        None
    }
}
