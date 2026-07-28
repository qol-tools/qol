use super::TracePlatformOps;

pub(super) struct Platform;

impl TracePlatformOps for Platform {
    fn process_name(&self, pid: &str) -> Option<String> {
        super::unix::process_name(pid)
    }

    fn initial_monitor_bounds(&self) -> Vec<(i64, i64, i64, i64)> {
        Vec::new()
    }
}
