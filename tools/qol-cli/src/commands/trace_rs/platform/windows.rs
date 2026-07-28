use super::TracePlatformOps;

pub(super) struct Platform;

impl TracePlatformOps for Platform {
    fn process_name(&self, _pid: &str) -> Option<String> {
        None
    }

    fn initial_monitor_bounds(&self) -> Vec<(i64, i64, i64, i64)> {
        Vec::new()
    }
}
