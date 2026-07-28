use tokio::net::UdpSocket;

pub(in crate::network) fn adopt_inherited_udp(_fd: i32) -> anyhow::Result<UdpSocket> {
    anyhow::bail!("PointZ inherited UDP sockets are unavailable on this platform")
}
