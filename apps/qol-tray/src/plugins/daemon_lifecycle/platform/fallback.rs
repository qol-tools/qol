use super::DaemonLifecyclePlatform;
use std::process::Command;

pub(super) struct Platform;

impl DaemonLifecyclePlatform for Platform {
    fn reaped_elsewhere(_error: &std::io::Error) -> bool {
        false
    }

    fn track_desktop_state_pid(_pid: u32) {}

    fn configure_process_group(_command: &mut Command) {}
}
