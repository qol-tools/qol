use tokio::net::UdpSocket;

pub(in crate::network) fn adopt_inherited_udp(fd: i32) -> anyhow::Result<UdpSocket> {
    super::unix_fd::adopt_inherited_udp(fd)
}
