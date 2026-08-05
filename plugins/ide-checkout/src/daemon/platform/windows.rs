use std::net::TcpListener;
use std::path::Path;

pub(in crate::daemon) fn is_executable(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|metadata| metadata.is_file())
}

pub(in crate::daemon) fn inherited_listener() -> std::io::Result<Option<TcpListener>> {
    Ok(None)
}

pub(in crate::daemon) fn spawn_host_death_watchdog() {}

pub(in crate::daemon) fn open_settings() -> Result<(), String> {
    Err("opening Task Runner settings is not supported on this platform".to_string())
}
