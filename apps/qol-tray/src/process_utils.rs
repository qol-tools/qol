use std::time::Duration;

pub use qol_process::{reap_children_nonblocking, terminate_owned};

pub fn is_pid_alive(pid: i32) -> bool {
    u32::try_from(pid).is_ok_and(qol_process::is_pid_alive)
}

pub fn terminate_pid(pid: i32, grace: Duration) {
    if let Ok(pid) = u32::try_from(pid) {
        qol_process::terminate_pid(pid, grace);
    }
}

pub fn terminate_group(pid: i32, grace: Duration) {
    if let Ok(pid) = u32::try_from(pid) {
        qol_process::terminate_group(pid, grace);
    }
}
