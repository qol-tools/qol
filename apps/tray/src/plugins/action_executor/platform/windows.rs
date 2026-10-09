use super::ActionProcessPlatform;

pub(super) struct Platform;

impl ActionProcessPlatform for Platform {
    fn track_desktop_state_pid(pid: u32) {
        crate::desktop_state::add_ignore_pid(pid);
    }

    fn untrack_desktop_state_pid(pid: u32) {
        crate::desktop_state::remove_ignore_pid(pid);
    }
}
