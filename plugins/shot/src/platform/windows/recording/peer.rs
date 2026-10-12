use std::net::{SocketAddr, SocketAddrV4};

use qol_process::TcpConnectionEntry;

pub(super) fn peer_process(peer: SocketAddr, local: SocketAddr) -> Option<u32> {
    let (SocketAddr::V4(peer), SocketAddr::V4(local)) = (peer, local) else {
        return None;
    };
    connection_owner(&qol_process::tcp_connections().ok()?, peer, local)
}

fn connection_owner(
    connections: &[TcpConnectionEntry],
    peer: SocketAddrV4,
    local: SocketAddrV4,
) -> Option<u32> {
    connections
        .iter()
        .find(|connection| connection.local == peer && connection.remote == local)
        .and_then(|connection| connection.pid)
}

#[cfg(test)]
mod tests {
    use super::{connection_owner, peer_process};
    use qol_process::TcpConnectionEntry;
    use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream};

    fn at(port: u16) -> SocketAddrV4 {
        SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)
    }

    #[test]
    fn the_owner_is_the_process_on_the_peer_side_of_the_connection() {
        let connections = [
            TcpConnectionEntry {
                local: at(4000),
                remote: at(5000),
                pid: Some(10),
            },
            TcpConnectionEntry {
                local: at(5000),
                remote: at(4000),
                pid: Some(20),
            },
            TcpConnectionEntry {
                local: at(5001),
                remote: at(4000),
                pid: Some(30),
            },
        ];
        let cases = [
            (at(5000), at(4000), Some(20)),
            (at(5001), at(4000), Some(30)),
            (at(4000), at(5000), Some(10)),
            (at(5002), at(4000), None),
            (at(5000), at(4001), None),
        ];
        for (peer, local, owner) in cases {
            assert_eq!(
                connection_owner(&connections, peer, local),
                owner,
                "{peer} {local}"
            );
        }
    }

    #[test]
    fn a_local_connection_is_owned_by_the_connecting_process() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server, peer) = listener.accept().unwrap();
        assert_eq!(peer, client.local_addr().unwrap());
        assert_eq!(
            peer_process(peer, server.local_addr().unwrap()),
            Some(std::process::id())
        );
    }
}
