use super::{DaemonTrackerPlatform, ManagedProcess};
use crate::plugins::Plugin;
use std::path::PathBuf;

pub(super) struct Platform;

pub(crate) struct ManagedRoots;

impl ManagedRoots {
    pub(crate) fn load() -> Self {
        Self
    }

    pub(crate) fn contains(&self, _target: &std::path::Path) -> bool {
        false
    }
}

impl DaemonTrackerPlatform for Platform {
    fn pid_exe_path(_pid: i32) -> Option<PathBuf> {
        None
    }

    fn kill_orphan_daemons() {}

    fn managed_processes() -> Vec<ManagedProcess> {
        Vec::new()
    }

    fn clean_stale_sockets(_plugins: &[Plugin]) {}

    fn kill_managed_process(process: &ManagedProcess, roots: &ManagedRoots) -> bool {
        roots.contains(&process.executable)
    }
}
