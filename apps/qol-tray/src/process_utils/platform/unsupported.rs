use std::time::Duration;

pub(super) fn is_pid_alive(_pid: i32) -> bool {
    false
}

pub(super) fn terminate_pid(_pid: i32, _grace: Duration) {}

pub(super) fn reap_children_nonblocking() {}
