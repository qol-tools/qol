use super::DaemonLifecyclePlatform;
use std::os::windows::process::CommandExt;
use std::process::Command;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub(super) struct Platform;

impl DaemonLifecyclePlatform for Platform {
    fn reaped_elsewhere(_error: &std::io::Error) -> bool {
        false
    }

    fn track_desktop_state_pid(pid: u32) {
        crate::desktop_state::add_ignore_pid(pid);
    }

    /// A console-subsystem daemon would otherwise open a console window of its own.
    fn configure_process_group(command: &mut Command) {
        command.creation_flags(CREATE_NO_WINDOW);
    }
}
