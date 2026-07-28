mod platform;

use crate::config::ServerConfig;
use if_addrs::get_if_addrs;
use std::net::IpAddr;
use tokio::net::UdpSocket;

pub(crate) struct InterfaceMetadata {
    pub name: String,
    pub address: IpAddr,
    pub loopback: bool,
}

pub(crate) struct NetworkMetadata {
    pub hostname: String,
    pub local_ipv4: Option<IpAddr>,
    pub interfaces: Vec<InterfaceMetadata>,
    pub interface_issue: Option<String>,
}

pub fn get_local_ip() -> Option<IpAddr> {
    get_if_addrs()
        .ok()?
        .iter()
        .find(|iface| !iface.is_loopback() && iface.ip().is_ipv4())
        .map(|iface| iface.ip())
}

pub fn get_hostname() -> String {
    gethostname::gethostname()
        .into_string()
        .unwrap_or_else(|_| ServerConfig::UNKNOWN_HOSTNAME.to_string())
}

pub(crate) fn inspect_metadata() -> NetworkMetadata {
    let hostname = get_hostname();
    let interfaces = match get_if_addrs() {
        Ok(interfaces) => interfaces,
        Err(error) => {
            return NetworkMetadata {
                hostname,
                local_ipv4: None,
                interfaces: Vec::new(),
                interface_issue: Some(error.to_string()),
            };
        }
    };
    let local_ipv4 = interfaces
        .iter()
        .find(|interface| !interface.is_loopback() && interface.ip().is_ipv4())
        .map(|interface| interface.ip());
    let mut interfaces = interfaces
        .into_iter()
        .map(|interface| {
            let address = interface.ip();
            let loopback = interface.is_loopback();
            InterfaceMetadata {
                name: interface.name,
                address,
                loopback,
            }
        })
        .collect::<Vec<_>>();
    interfaces.sort_by(|left, right| {
        (&left.name, left.address.to_string()).cmp(&(&right.name, right.address.to_string()))
    });
    NetworkMetadata {
        hostname,
        local_ipv4,
        interfaces,
        interface_issue: None,
    }
}

/// Adopts the UDP socket qol-tray pre-bound for the named port (matching a
/// `[[daemon.extra_ports]]` entry in plugin.toml), or binds it directly if
/// qol-tray didn't pre-bind it (e.g. when run outside qol-tray's supervision).
pub async fn bind_udp_or_inherit(name: &str, port: u16) -> anyhow::Result<UdpSocket> {
    if let Some(fd) = qol_plugin_daemon::daemon::inherited_port_fd(name) {
        return platform::adopt_inherited_udp(fd);
    }
    Ok(UdpSocket::bind(("0.0.0.0", port)).await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::fd::IntoRawFd;

    // Each test below uses its own distinct env var suffix
    // (_TESTFALLBACK / _TESTINHERIT), so unlike qol-plugin-daemon's tests for
    // the single shared QOL_TRAY_DAEMON_LISTENER_FD name, there's no
    // cross-test collision to guard with a lock here.

    #[tokio::test]
    async fn bind_udp_or_inherit_binds_directly_when_env_var_absent() {
        let env_name = format!("{}_TESTFALLBACK", qol_conventions::ENV_DAEMON_PORT_FD);
        std::env::remove_var(&env_name);

        let socket = bind_udp_or_inherit("testfallback", 0).await;

        assert!(
            socket.is_ok(),
            "must bind directly when nothing is pre-bound"
        );
    }

    // Regression test: tokio::net::UdpSocket::from_std requires the socket to
    // already be in non-blocking mode, which an fd inherited via
    // std::net::UdpSocket::from_raw_fd is not by default. Forgetting
    // set_nonblocking here previously panicked at daemon startup the moment
    // qol-tray pre-bound a port for this plugin.
    #[tokio::test]
    #[cfg(unix)]
    async fn bind_udp_or_inherit_adopts_an_inherited_fd_without_panicking() {
        let pre_bound = std::net::UdpSocket::bind(("127.0.0.1", 0)).unwrap();
        let fd = pre_bound.into_raw_fd();
        let env_name = format!("{}_TESTINHERIT", qol_conventions::ENV_DAEMON_PORT_FD);
        std::env::set_var(&env_name, fd.to_string());

        let result = bind_udp_or_inherit("testinherit", 0).await;

        std::env::remove_var(&env_name);
        let socket = result.expect(
            "adopting an inherited fd must not panic - it must be set non-blocking \
             before tokio::net::UdpSocket::from_std, or tokio's reactor registration fails",
        );

        let sender = tokio::net::UdpSocket::bind(("127.0.0.1", 0)).await.unwrap();
        let target = socket.local_addr().unwrap();
        sender.send_to(b"ping", target).await.unwrap();
        let mut buf = [0u8; 4];
        let (size, _) = socket.recv_from(&mut buf).await.unwrap();
        assert_eq!(
            &buf[..size],
            b"ping",
            "the adopted socket must be genuinely usable for async I/O"
        );
    }
}
