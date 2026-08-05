use std::net::TcpListener;
use std::os::fd::FromRawFd;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

pub(in crate::daemon) fn is_executable(path: &Path) -> bool {
    std::fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

pub(in crate::daemon) fn inherited_listener() -> std::io::Result<Option<TcpListener>> {
    let Some(fd) = qol_plugin_daemon::daemon::inherited_primary_port_fd() else {
        return Ok(None);
    };
    qol_plugin_daemon::daemon::restore_cloexec(fd)?;
    Ok(Some(unsafe { TcpListener::from_raw_fd(fd) }))
}

pub(in crate::daemon) fn spawn_host_death_watchdog() {
    qol_runtime::spawn_host_death_watchdog();
}

pub(in crate::daemon) fn open_settings() -> Result<(), String> {
    let url = qol_conventions::settings_url(env!("QOL_PLUGIN_ID"));
    let launcher = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(launcher)
        .arg(url)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Failed to open settings page: {error}"))
}
