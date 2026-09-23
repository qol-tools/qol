use super::ActionProcessPlatform;

pub(super) struct Platform;

impl ActionProcessPlatform for Platform {
    fn track_desktop_state_pid(_pid: u32) {}

    fn untrack_desktop_state_pid(_pid: u32) {}
}
