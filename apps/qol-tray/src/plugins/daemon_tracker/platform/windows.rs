use crate::plugins::Plugin;
use std::path::PathBuf;

pub fn pid_exe_path(_pid: i32) -> Option<PathBuf> {
    None
}

pub fn kill_orphan_daemons() {}

pub fn clean_stale_sockets(_plugins: &[Plugin]) {}
