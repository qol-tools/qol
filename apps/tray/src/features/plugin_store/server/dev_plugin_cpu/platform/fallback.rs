use super::DevPluginCpuPlatformOps;

pub(super) struct Platform;

impl DevPluginCpuPlatformOps for Platform {
    fn cpu_percent_window_samples() -> usize {
        1
    }

    fn process_cpu_micros(_pid: i32) -> Option<u64> {
        None
    }
}
