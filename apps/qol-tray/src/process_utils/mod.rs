use std::time::Duration;

mod platform;

pub fn is_pid_alive(pid: i32) -> bool {
    platform::is_pid_alive(pid)
}

pub fn terminate_pid(pid: i32, grace: Duration) {
    platform::terminate_pid(pid, grace);
}

pub fn reap_children_nonblocking() {
    platform::reap_children_nonblocking();
}
