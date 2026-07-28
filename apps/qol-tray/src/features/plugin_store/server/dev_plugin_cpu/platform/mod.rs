mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "windows")]
use fallback as imp;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
use fallback as imp;
#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "macos")]
use macos as imp;

trait DevPluginCpuPlatformOps {
    fn cpu_percent_window_samples() -> usize;
    fn process_cpu_micros(pid: i32) -> Option<u64>;
}

pub(super) fn cpu_percent_window_samples() -> usize {
    imp::Platform::cpu_percent_window_samples()
}

pub(super) fn process_cpu_micros(pid: i32) -> Option<u64> {
    imp::Platform::process_cpu_micros(pid)
}

const _: fallback::Platform = fallback::Platform;
