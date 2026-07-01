use std::io;
use std::process::{Child, ExitStatus};

pub(crate) enum TrayHandle {
    Owned(Child),
    Attached(u32),
}

impl TrayHandle {
    pub(crate) fn id(&self) -> u32 {
        match self {
            Self::Owned(child) => child.id(),
            Self::Attached(pid) => *pid,
        }
    }

    pub(crate) fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        match self {
            Self::Owned(child) => child.try_wait(),
            Self::Attached(pid) => platform::try_wait_pid(*pid),
        }
    }

    pub(crate) fn wait(&mut self) -> io::Result<ExitStatus> {
        match self {
            Self::Owned(child) => child.wait(),
            Self::Attached(pid) => platform::wait_pid(*pid),
        }
    }

    pub(crate) fn kill(&mut self) -> io::Result<()> {
        match self {
            Self::Owned(child) => child.kill(),
            Self::Attached(pid) => platform::kill_pid(*pid),
        }
    }

    pub(crate) fn signal_term(&self) {
        match self {
            Self::Owned(child) => platform::signal_term_pid(child.id()),
            Self::Attached(pid) => platform::signal_term_pid(*pid),
        }
    }
}

#[cfg(unix)]
mod platform {
    use std::io;
    use std::os::unix::process::ExitStatusExt;
    use std::process::ExitStatus;
    use std::time::Duration;

    pub(super) fn try_wait_pid(pid: u32) -> io::Result<Option<ExitStatus>> {
        let mut status = 0;
        let result = unsafe { libc::waitpid(pid as libc::pid_t, &mut status, libc::WNOHANG) };
        match result {
            0 => Ok(None),
            n if n == pid as libc::c_int => Ok(Some(ExitStatus::from_raw(status))),
            _ => Err(io::Error::last_os_error()),
        }
    }

    pub(super) fn wait_pid(pid: u32) -> io::Result<ExitStatus> {
        loop {
            if let Some(status) = try_wait_pid(pid)? {
                return Ok(status);
            }
            if unsafe { libc::kill(pid as libc::pid_t, 0) } != 0 {
                return Ok(ExitStatus::from_raw(0));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    pub(super) fn kill_pid(pid: u32) -> io::Result<()> {
        if unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) } == 0 {
            return Ok(());
        }
        Err(io::Error::last_os_error())
    }

    pub(super) fn signal_term_pid(pid: u32) {
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGTERM);
        }
    }
}

#[cfg(windows)]
mod platform {
    use std::io;
    use std::os::windows::process::ExitStatusExt;
    use std::process::{Command, ExitStatus};
    use std::time::Duration;

    pub(super) fn try_wait_pid(pid: u32) -> io::Result<Option<ExitStatus>> {
        if pid_exists(pid) {
            return Ok(None);
        }
        Ok(Some(ExitStatus::from_raw(0)))
    }

    pub(super) fn wait_pid(pid: u32) -> io::Result<ExitStatus> {
        while pid_exists(pid) {
            std::thread::sleep(Duration::from_millis(50));
        }
        Ok(ExitStatus::from_raw(0))
    }

    pub(super) fn kill_pid(pid: u32) -> io::Result<()> {
        let status = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .status()?;
        if status.success() {
            return Ok(());
        }
        Err(io::Error::other("taskkill did not exit successfully"))
    }

    pub(super) fn signal_term_pid(pid: u32) {
        let _ = kill_pid(pid);
    }

    fn pid_exists(pid: u32) -> bool {
        let Ok(output) = Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
        else {
            return false;
        };
        String::from_utf8_lossy(&output.stdout).contains(&pid.to_string())
    }
}

#[cfg(all(test, unix))]
#[allow(clippy::zombie_processes)]
mod tests {
    // These tests reap the spawned process through TrayHandle::Attached's own
    // waitpid-based wait()/kill(), not through the Child binding directly -
    // that's the exact behavior under test, but clippy can't trace it.
    use super::*;
    use std::process::Command;

    fn spawn_sleep(seconds: u32) -> Child {
        Command::new("sleep")
            .arg(seconds.to_string())
            .spawn()
            .expect("failed to spawn sleep")
    }

    #[test]
    fn attached_try_wait_reports_none_while_running() {
        let child = spawn_sleep(5);
        let pid = child.id();
        let mut handle = TrayHandle::Attached(pid);

        assert_eq!(
            handle.try_wait().unwrap(),
            None,
            "a still-running attached pid must report None, not an exit status"
        );

        handle.kill().unwrap();
        handle.wait().unwrap();
    }

    #[test]
    fn attached_wait_blocks_until_exit_and_reaps() {
        let child = spawn_sleep(0);
        let pid = child.id();
        let mut handle = TrayHandle::Attached(pid);

        let status = handle.wait().unwrap();
        assert!(
            status.success(),
            "sleep 0 should exit successfully, got: {status:?}"
        );
    }

    #[test]
    fn attached_kill_terminates_a_running_process() {
        let child = spawn_sleep(30);
        let pid = child.id();
        let mut handle = TrayHandle::Attached(pid);

        handle.kill().unwrap();
        let status = handle.wait().unwrap();
        assert!(
            !status.success(),
            "a killed process must not report a successful exit"
        );
    }

    #[test]
    fn owned_and_attached_report_the_same_pid() {
        let child = spawn_sleep(0);
        let pid = child.id();
        let mut owned = TrayHandle::Owned(child);
        assert_eq!(owned.id(), pid);
        owned.wait().unwrap();

        let attached = TrayHandle::Attached(pid);
        assert_eq!(attached.id(), pid);
    }
}
