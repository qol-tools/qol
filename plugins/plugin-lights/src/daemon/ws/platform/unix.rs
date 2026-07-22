use std::net::TcpListener;
use std::os::fd::FromRawFd;

pub(crate) fn bind_listener(port: u16) -> std::io::Result<TcpListener> {
    if let Some(fd) = qol_plugin_daemon::daemon::inherited_primary_port_fd() {
        qol_plugin_daemon::daemon::restore_cloexec(fd)?;
        return Ok(unsafe { TcpListener::from_raw_fd(fd) });
    }
    TcpListener::bind(format!("127.0.0.1:{port}"))
}
