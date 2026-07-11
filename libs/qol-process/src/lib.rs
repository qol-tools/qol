mod platform;

use std::io;
use std::process::{Child, ExitStatus};
use std::time::Duration;

pub fn is_pid_alive(pid: u32) -> bool {
    platform::is_pid_alive(pid)
}

pub fn signal_term_pid(pid: u32) -> io::Result<()> {
    platform::signal_term_pid(pid)
}

pub fn kill_pid(pid: u32) -> io::Result<()> {
    platform::kill_pid(pid)
}

pub fn try_wait_pid(pid: u32) -> io::Result<Option<ExitStatus>> {
    platform::try_wait_pid(pid)
}

pub fn wait_pid(pid: u32) -> io::Result<ExitStatus> {
    platform::wait_pid(pid)
}

pub fn terminate_pid(pid: u32, grace: Duration) {
    platform::terminate_pid(pid, grace);
}

pub fn terminate_group(pid: u32, grace: Duration) {
    platform::terminate_group(pid, grace);
}

pub fn terminate_owned(child: &mut Child, grace: Duration) -> io::Result<()> {
    platform::terminate_owned(child, grace)
}

pub fn reap_children_nonblocking() {
    platform::reap_children_nonblocking();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_process_is_alive() {
        assert!(is_pid_alive(std::process::id()));
    }

    #[test]
    fn zero_pid_is_never_a_process_target() {
        assert!(!is_pid_alive(0));
        assert!(signal_term_pid(0).is_err());
        assert!(kill_pid(0).is_err());
        assert!(try_wait_pid(0).is_err());
        assert!(wait_pid(0).is_err());
    }
}
