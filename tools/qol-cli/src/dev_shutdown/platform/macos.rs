use super::pid_files;
use super::DevShutdownPlatform;
use crate::dev_shutdown::TrackedDaemonPid;
use std::process::Command;

pub(crate) struct Platform;

impl DevShutdownPlatform for Platform {
    fn snapshot_runtime_daemon_pids(&self) -> Vec<TrackedDaemonPid> {
        pid_files::tracked_pids_from_dir(&pid_files::runtime_pids_dir())
            .into_iter()
            .filter(|daemon| self.group_is_owned(daemon))
            .collect()
    }

    fn group_is_owned(&self, daemon: &TrackedDaemonPid) -> bool {
        qol_process::is_group_alive(daemon.pid)
    }

    fn configure_tray_child(&self, _command: &mut Command) -> std::io::Result<()> {
        Ok(())
    }
}
