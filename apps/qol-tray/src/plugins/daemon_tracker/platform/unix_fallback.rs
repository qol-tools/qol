use super::{DaemonTrackerPlatform, ManagedProcess, ManagedRoots};
use crate::plugins::Plugin;
use std::path::PathBuf;

pub(super) struct Platform;

impl DaemonTrackerPlatform for Platform {
    fn pid_exe_path(_pid: i32) -> Option<PathBuf> {
        None
    }

    fn kill_orphan_daemons() {}

    fn managed_processes() -> Vec<ManagedProcess> {
        Vec::new()
    }

    fn clean_stale_sockets(_plugins: &[Plugin]) {}

    fn kill_managed_process(_process: &ManagedProcess, _roots: &ManagedRoots) -> bool {
        false
    }
}
