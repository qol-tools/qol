use std::os::fd::FromRawFd;

use tokio::net::UdpSocket;

pub(super) fn adopt_inherited_udp(fd: i32) -> anyhow::Result<UdpSocket> {
    qol_plugin_daemon::daemon::restore_cloexec(fd)?;
    let socket = unsafe { std::net::UdpSocket::from_raw_fd(fd) };
    socket.set_nonblocking(true)?;
    Ok(UdpSocket::from_std(socket)?)
}
