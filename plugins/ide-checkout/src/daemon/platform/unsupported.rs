use std::net::TcpListener;
use std::path::{Path, PathBuf};

pub(in crate::daemon) fn is_executable(_path: &Path) -> bool {
    false
}

pub(in crate::daemon) fn executable_candidates(directory: &Path, program: &str) -> Vec<PathBuf> {
    vec![directory.join(program)]
}

pub(in crate::daemon) fn launch_path(path: &Path) -> std::io::Result<PathBuf> {
    std::fs::canonicalize(path)
}

pub(in crate::daemon) fn inherited_listener() -> std::io::Result<Option<TcpListener>> {
    Ok(None)
}
