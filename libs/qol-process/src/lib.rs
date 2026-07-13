mod platform;

use std::io;
use std::process::{Child, Command, ExitStatus};
use std::time::Duration;

pub fn is_pid_alive(pid: u32) -> bool {
    platform::is_pid_alive(pid)
}

pub fn is_group_alive(pid: u32) -> bool {
    platform::is_group_alive(pid)
}

pub fn is_pid_zombie(pid: u32) -> bool {
    platform::is_pid_zombie(pid)
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

/// Spawns a command without inherited standard streams or a parent-owned child
/// process that needs later cleanup.
pub fn spawn_detached(command: &mut Command) -> io::Result<()> {
    platform::spawn_detached(command)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn current_process_is_alive() {
        assert!(is_pid_alive(std::process::id()));
    }

    #[test]
    fn zero_pid_is_never_a_process_target() {
        assert!(!is_pid_alive(0));
        assert!(!is_group_alive(0));
        assert!(!is_pid_zombie(0));
        assert!(signal_term_pid(0).is_err());
        assert!(kill_pid(0).is_err());
        assert!(try_wait_pid(0).is_err());
        assert!(wait_pid(0).is_err());
    }

    #[cfg(unix)]
    #[test]
    #[allow(clippy::zombie_processes)]
    fn terminate_group_stops_the_leader_and_descendants() {
        use std::os::unix::process::CommandExt;

        let mut command = Command::new("sh");
        command.args(["-c", "sleep 30 & wait"]);
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = command.spawn().unwrap();
        let pid = child.id();
        assert!(is_group_alive(pid));

        terminate_group(pid, Duration::from_secs(1));

        assert!(!is_group_alive(pid));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn exited_unreaped_process_is_a_zombie_until_waited() {
        let mut child = Command::new("sh").args(["-c", "exit 0"]).spawn().unwrap();
        let pid = child.id();
        let deadline = Instant::now() + Duration::from_secs(1);
        while !is_pid_zombie(pid) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }

        assert!(is_pid_zombie(pid));
        child.wait().unwrap();
        assert!(!is_pid_zombie(pid));
    }

    #[test]
    fn detached_child_helper() {
        let Some(marker) = std::env::var_os("QOL_PROCESS_DETACHED_TEST_MARKER") else {
            return;
        };
        std::fs::write(marker, "ready").unwrap();
    }

    #[test]
    fn detached_spawn_runs_after_the_owned_child_is_released() {
        let temp = tempfile::tempdir().unwrap();
        let marker = temp.path().join("detached-ready");
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", "tests::detached_child_helper"])
            .env("QOL_PROCESS_DETACHED_TEST_MARKER", &marker);

        spawn_detached(&mut command).unwrap();

        let deadline = Instant::now() + Duration::from_secs(2);
        while !marker.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(std::fs::read_to_string(marker).unwrap(), "ready");
    }

    #[test]
    fn detached_spawn_reports_a_missing_program() {
        let mut command = Command::new("qol-process-command-that-does-not-exist");
        assert_eq!(
            spawn_detached(&mut command).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
    }
}
