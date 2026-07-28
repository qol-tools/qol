use super::DaemonSupervision;
use crate::dev_shutdown::TrackedDaemonPid;

pub(crate) struct Platform;

impl DaemonSupervision for Platform {
    fn snapshot_runtime_daemon_pids(&self) -> Vec<TrackedDaemonPid> {
        Vec::new()
    }

    fn group_is_owned(&self, daemon: &TrackedDaemonPid) -> bool {
        qol_process::is_pid_alive(daemon.pid)
    }
}
