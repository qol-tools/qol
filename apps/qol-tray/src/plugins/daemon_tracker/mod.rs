use super::Plugin;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManagedProcess {
    pub pid: i32,
    pub executable: PathBuf,
}

mod census;
mod identity;
pub mod platform;
mod reaper;
pub(crate) mod registry;

pub use census::{leaked_processes, managed_processes};
pub use identity::running_exe_path;
pub use reaper::{kill_managed_processes, kill_orphan_daemons};

pub(crate) use identity::ManagedRoots;
#[cfg(unix)]
pub(crate) use reaper::kill_from_pid_files;

pub fn clean_stale_sockets(plugins: &[Plugin]) {
    platform::clean_stale_sockets(plugins);
}
