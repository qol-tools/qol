use super::pid_files;
use super::DaemonSupervision;
use crate::dev_shutdown::TrackedDaemonPid;

pub(crate) struct Platform;

impl DaemonSupervision for Platform {
    fn snapshot_runtime_daemon_pids(&self) -> Vec<TrackedDaemonPid> {
        pid_files::tracked_pids_from_dir(&pid_files::runtime_pids_dir())
            .into_iter()
            .filter(|daemon| self.group_is_owned(daemon))
            .collect()
    }

    fn group_is_owned(&self, daemon: &TrackedDaemonPid) -> bool {
        qol_process::is_group_alive(daemon.pid)
    }
}
