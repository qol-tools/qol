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
pub(crate) use platform::ManagedRoots;
pub use reaper::{kill_managed_processes, kill_orphan_daemons};

pub fn clean_stale_sockets(plugins: &[Plugin]) {
    platform::clean_stale_sockets(plugins);
}
