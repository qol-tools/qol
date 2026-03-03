#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

pub(super) fn cpu_percent_window_samples() -> usize {
    #[cfg(target_os = "linux")]
    {
        return linux::cpu_percent_window_samples();
    }

    #[cfg(target_os = "macos")]
    {
        return macos::cpu_percent_window_samples();
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        1
    }
}

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
