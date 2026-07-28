use std::net::TcpListener;
use std::path::Path;

pub(in crate::daemon) fn is_executable(_path: &Path) -> bool {
    false
}

pub(in crate::daemon) fn inherited_listener() -> std::io::Result<Option<TcpListener>> {
    Ok(None)
}

pub(in crate::daemon) fn spawn_host_death_watchdog() {}
