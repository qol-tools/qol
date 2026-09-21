pub(super) fn process_holds_handoff_resources(pid: u32) -> bool {
    crate::process_utils::is_pid_alive(pid as i32)
}
