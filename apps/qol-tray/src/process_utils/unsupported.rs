use std::time::Duration;

pub fn is_pid_alive(_pid: i32) -> bool {
    false
}

pub fn terminate_pid(_pid: i32, _grace: Duration) {}

pub fn reap_children_nonblocking() {}
