use super::DaemonLifecyclePlatform;
use std::process::Command;

pub(super) struct Platform;

impl DaemonLifecyclePlatform for Platform {
    fn reaped_elsewhere(_error: &std::io::Error) -> bool {
        false
    }

    /// A console-subsystem daemon would otherwise open a console window of its own.
    fn configure_process_group(command: &mut Command) {
        qol_process::hide_console_window(command);
    }
}
