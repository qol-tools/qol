use super::DevShutdownPlatform;
use crate::dev_shutdown::TrackedDaemonPid;
use std::process::Command;

pub(crate) struct Platform;

impl DevShutdownPlatform for Platform {
    fn snapshot_runtime_daemon_pids(&self) -> Vec<TrackedDaemonPid> {
        Vec::new()
    }

    fn group_is_owned(&self, daemon: &TrackedDaemonPid) -> bool {
        qol_process::is_pid_alive(daemon.pid)
    }

    fn configure_tray_child(&self, _command: &mut Command) -> std::io::Result<()> {
        Ok(())
    }
}
