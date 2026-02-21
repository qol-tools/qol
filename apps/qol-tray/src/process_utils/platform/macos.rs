use std::time::Duration;

pub(super) fn is_pid_alive(pid: i32) -> bool {
    super::unix_common::is_pid_alive(pid)
}

pub(super) fn terminate_pid(pid: i32, grace: Duration) {
    super::unix_common::terminate_pid(pid, grace)
}

pub(super) fn reap_children_nonblocking() {
    super::unix_common::reap_children_nonblocking();
}
