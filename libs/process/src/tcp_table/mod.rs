use std::net::{IpAddr, SocketAddrV4};

mod platform;

pub use platform::{tcp_connections, tcp_listeners};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TcpListenerEntry {
    pub address: IpAddr,
    pub port: u16,
    pub pid: Option<u32>,
    pub uid: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TcpConnectionEntry {
    pub local: SocketAddrV4,
    pub remote: SocketAddrV4,
    pub pid: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    #[test]
    fn a_socket_this_process_listens_on_is_listed_with_its_pid() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let listed = tcp_listeners().unwrap();

        assert!(
            listed
                .iter()
                .any(|entry| entry.address == std::net::Ipv4Addr::LOCALHOST
                    && entry.port == port
                    && entry.pid == Some(std::process::id())),
            "port {port} missing from {listed:?}"
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    #[test]
    fn a_local_connection_is_listed_on_both_ends_with_this_process() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let client = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (_server, _) = listener.accept().unwrap();
        let std::net::SocketAddr::V4(client_end) = client.local_addr().unwrap() else {
            panic!("an IPv4 connection has IPv4 ends");
        };
        let std::net::SocketAddr::V4(server_end) = client.peer_addr().unwrap() else {
            panic!("an IPv4 connection has IPv4 ends");
        };

        let connections = tcp_connections().unwrap();

        for (local, remote) in [(client_end, server_end), (server_end, client_end)] {
            assert!(
                connections.iter().any(|entry| entry.local == local
                    && entry.remote == remote
                    && entry.pid == Some(std::process::id())),
                "{local} -> {remote} missing from {connections:?}"
            );
        }
    }
}
