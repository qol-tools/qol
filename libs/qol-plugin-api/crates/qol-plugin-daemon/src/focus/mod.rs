mod platform;

pub fn should_poll_process_focus() -> bool {
    platform::should_poll_process_focus()
}

pub fn has_process_focus() -> bool {
    platform::has_process_focus()
}
