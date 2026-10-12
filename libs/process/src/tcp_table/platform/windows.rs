use std::io;
use std::mem::offset_of;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, NO_ERROR};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    GetExtendedTcpTable, MIB_TCP6ROW_OWNER_PID, MIB_TCP6TABLE_OWNER_PID, MIB_TCPROW_OWNER_PID,
    MIB_TCPTABLE_OWNER_PID, TCP_TABLE_CLASS, TCP_TABLE_OWNER_PID_CONNECTIONS,
    TCP_TABLE_OWNER_PID_LISTENER,
};
use windows_sys::Win32::Networking::WinSock::{AF_INET, AF_INET6};

use super::super::{TcpConnectionEntry, TcpListenerEntry};

const TABLE_READ_ATTEMPTS: usize = 3;

pub fn tcp_listeners() -> io::Result<Vec<TcpListenerEntry>> {
    let ipv4 = ipv4_rows(TCP_TABLE_OWNER_PID_LISTENER)?
        .into_iter()
        .map(|row| listener(row.dwLocalPort, row.dwOwningPid));
    let ipv6 = owner_pid_rows::<MIB_TCP6ROW_OWNER_PID>(
        AF_INET6,
        TCP_TABLE_OWNER_PID_LISTENER,
        offset_of!(MIB_TCP6TABLE_OWNER_PID, table),
    )?
    .into_iter()
    .map(|row| listener(row.dwLocalPort, row.dwOwningPid));
    Ok(ipv4.chain(ipv6).collect())
}

pub fn tcp_connections() -> io::Result<Vec<TcpConnectionEntry>> {
    Ok(ipv4_rows(TCP_TABLE_OWNER_PID_CONNECTIONS)?
        .into_iter()
        .map(|row| TcpConnectionEntry {
            local: endpoint(row.dwLocalAddr, row.dwLocalPort),
            remote: endpoint(row.dwRemoteAddr, row.dwRemotePort),
            pid: Some(row.dwOwningPid),
        })
        .collect())
}

fn listener(local_port: u32, pid: u32) -> TcpListenerEntry {
    TcpListenerEntry {
        port: table_port(local_port),
        pid: Some(pid),
        uid: None,
    }
}

fn endpoint(addr: u32, port: u32) -> SocketAddrV4 {
    SocketAddrV4::new(Ipv4Addr::from(addr.to_ne_bytes()), table_port(port))
}

fn table_port(port: u32) -> u16 {
    u16::from_be(port as u16)
}

fn ipv4_rows(class: TCP_TABLE_CLASS) -> io::Result<Vec<MIB_TCPROW_OWNER_PID>> {
    owner_pid_rows(AF_INET, class, offset_of!(MIB_TCPTABLE_OWNER_PID, table))
}

fn owner_pid_rows<Row: Copy>(
    family: u16,
    class: TCP_TABLE_CLASS,
    rows_offset: usize,
) -> io::Result<Vec<Row>> {
    let family = u32::from(family);
    for _ in 0..TABLE_READ_ATTEMPTS {
        let mut size = 0u32;
        let probe = unsafe { GetExtendedTcpTable(null_mut(), &mut size, 0, family, class, 0) };
        if probe != ERROR_INSUFFICIENT_BUFFER && probe != NO_ERROR {
            return Err(io::Error::from_raw_os_error(probe as i32));
        }
        let mut buffer = vec![0u64; (size as usize).div_ceil(8) + 1];
        let mut size = (buffer.len() * 8) as u32;
        let status = unsafe {
            GetExtendedTcpTable(buffer.as_mut_ptr().cast(), &mut size, 0, family, class, 0)
        };
        if status == ERROR_INSUFFICIENT_BUFFER {
            continue;
        }
        if status != NO_ERROR {
            return Err(io::Error::from_raw_os_error(status as i32));
        }
        let base = buffer.as_ptr().cast::<u8>();
        let count = unsafe { base.cast::<u32>().read_unaligned() } as usize;
        let capacity = (buffer.len() * 8).saturating_sub(rows_offset) / std::mem::size_of::<Row>();
        return Ok((0..count.min(capacity))
            .map(|index| unsafe {
                base.add(rows_offset + index * std::mem::size_of::<Row>())
                    .cast::<Row>()
                    .read_unaligned()
            })
            .collect());
    }
    Err(io::Error::other(
        "the TCP table kept growing while it was read",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_endpoints_read_network_order_addresses_and_ports() {
        let addr = u32::from_ne_bytes([127, 0, 0, 1]);
        let port = u32::from(8080u16.to_be());
        assert_eq!(
            endpoint(addr, port),
            SocketAddrV4::new(Ipv4Addr::LOCALHOST, 8080)
        );
    }
}
