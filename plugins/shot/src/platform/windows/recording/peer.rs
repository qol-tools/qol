use std::ffi::c_void;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

use windows_sys::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, NO_ERROR};

const AF_INET: u32 = 2;
const TCP_TABLE_OWNER_PID_CONNECTIONS: i32 = 4;
const TABLE_ROWS_OFFSET: usize = std::mem::size_of::<u32>();
const TABLE_READ_ATTEMPTS: usize = 3;

#[link(name = "iphlpapi")]
extern "system" {
    fn GetExtendedTcpTable(
        table: *mut c_void,
        size: *mut u32,
        order: i32,
        family: u32,
        class: i32,
        reserved: u32,
    ) -> u32;
}

#[repr(C)]
#[derive(Clone, Copy)]
struct OwnerPidRow {
    _state: u32,
    local_addr: u32,
    local_port: u32,
    remote_addr: u32,
    remote_port: u32,
    owning_pid: u32,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct Connection {
    local: SocketAddrV4,
    remote: SocketAddrV4,
    pid: u32,
}

pub(super) fn peer_process(peer: SocketAddr, local: SocketAddr) -> Option<u32> {
    let (SocketAddr::V4(peer), SocketAddr::V4(local)) = (peer, local) else {
        return None;
    };
    connection_owner(&connections()?, peer, local)
}

fn connection_owner(
    connections: &[Connection],
    peer: SocketAddrV4,
    local: SocketAddrV4,
) -> Option<u32> {
    connections
        .iter()
        .find(|connection| connection.local == peer && connection.remote == local)
        .map(|connection| connection.pid)
}

fn connections() -> Option<Vec<Connection>> {
    let rows = owner_pid_rows()?;
    Some(
        rows.iter()
            .map(|row| Connection {
                local: endpoint(row.local_addr, row.local_port),
                remote: endpoint(row.remote_addr, row.remote_port),
                pid: row.owning_pid,
            })
            .collect(),
    )
}

fn endpoint(addr: u32, port: u32) -> SocketAddrV4 {
    SocketAddrV4::new(
        Ipv4Addr::from(addr.to_ne_bytes()),
        u16::from_be(port as u16),
    )
}

fn owner_pid_rows() -> Option<Vec<OwnerPidRow>> {
    let mut size = 0u32;
    for _ in 0..TABLE_READ_ATTEMPTS {
        let mut buffer = vec![0u64; (size as usize).div_ceil(8) + 1];
        size = (buffer.len() * 8) as u32;
        let status = unsafe {
            GetExtendedTcpTable(
                buffer.as_mut_ptr().cast(),
                &mut size,
                0,
                AF_INET,
                TCP_TABLE_OWNER_PID_CONNECTIONS,
                0,
            )
        };
        if status == ERROR_INSUFFICIENT_BUFFER {
            continue;
        }
        if status != NO_ERROR {
            return None;
        }
        let base = buffer.as_ptr().cast::<u8>();
        let count = unsafe { base.cast::<u32>().read_unaligned() } as usize;
        let row_size = std::mem::size_of::<OwnerPidRow>();
        let capacity = (buffer.len() * 8).saturating_sub(TABLE_ROWS_OFFSET) / row_size;
        return Some(
            (0..count.min(capacity))
                .map(|index| unsafe {
                    base.add(TABLE_ROWS_OFFSET + index * row_size)
                        .cast::<OwnerPidRow>()
                        .read_unaligned()
                })
                .collect(),
        );
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{connection_owner, endpoint, peer_process, Connection};
    use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream};

    fn at(port: u16) -> SocketAddrV4 {
        SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)
    }

    #[test]
    fn the_owner_is_the_process_on_the_peer_side_of_the_connection() {
        let connections = [
            Connection {
                local: at(4000),
                remote: at(5000),
                pid: 10,
            },
            Connection {
                local: at(5000),
                remote: at(4000),
                pid: 20,
            },
            Connection {
                local: at(5001),
                remote: at(4000),
                pid: 30,
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
    fn table_endpoints_read_network_order_addresses_and_ports() {
        let addr = u32::from_ne_bytes([127, 0, 0, 1]);
        let port = u32::from(8080u16.to_be());
        assert_eq!(endpoint(addr, port), at(8080));
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
