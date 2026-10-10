use std::net::TcpListener;
use std::os::fd::FromRawFd;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

pub(in crate::daemon) fn is_executable(path: &Path) -> bool {
    std::fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

pub(in crate::daemon) fn executable_candidates(directory: &Path, program: &str) -> Vec<PathBuf> {
    vec![directory.join(program)]
}

pub(in crate::daemon) fn launch_path(path: &Path) -> std::io::Result<PathBuf> {
    std::fs::canonicalize(path)
}

pub(in crate::daemon) fn inherited_listener() -> std::io::Result<Option<TcpListener>> {
    let Some(fd) = qol_plugin_daemon::daemon::inherited_primary_port_fd() else {
        return Ok(None);
    };
    qol_plugin_daemon::daemon::restore_cloexec(fd)?;
    Ok(Some(unsafe { TcpListener::from_raw_fd(fd) }))
}
