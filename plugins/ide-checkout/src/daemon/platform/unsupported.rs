use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;

pub(in crate::daemon) fn is_executable(_path: &Path) -> bool {
    false
}

pub(in crate::daemon) fn executable_candidates(directory: &Path, program: &str) -> Vec<PathBuf> {
    vec![directory.join(program)]
}

pub(in crate::daemon) fn launch_path(path: &Path) -> std::io::Result<PathBuf> {
    std::fs::canonicalize(path)
}

pub(in crate::daemon) fn hide_console(_command: &mut Command) {}

pub(in crate::daemon) fn inherited_listener() -> std::io::Result<Option<TcpListener>> {
    Ok(None)
}

pub(in crate::daemon) fn spawn_host_death_watchdog() {}
