use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::path::Path;

use super::super::{TcpConnectionEntry, TcpListenerEntry};

const IPV4_TABLE: &str = "/proc/net/tcp";
const TABLES: [&str; 2] = [IPV4_TABLE, "/proc/net/tcp6"];
const LISTEN_STATE: &str = "0A";

struct Socket {
    port: u16,
    uid: u32,
    inode: u64,
}

struct Connection {
    local: SocketAddrV4,
    remote: SocketAddrV4,
    inode: u64,
}

pub fn tcp_listeners() -> io::Result<Vec<TcpListenerEntry>> {
    let mut sockets = Vec::new();
    for table in TABLES {
        if let Some(text) = read_table(table)? {
            sockets.extend(listening_sockets(&text));
        }
    }
    let owners = socket_owners(&sockets.iter().map(|socket| socket.inode).collect());
    Ok(sockets
        .into_iter()
        .map(|socket| TcpListenerEntry {
            port: socket.port,
            pid: owners.get(&socket.inode).copied(),
            uid: Some(socket.uid),
        })
        .collect())
}

pub fn tcp_connections() -> io::Result<Vec<TcpConnectionEntry>> {
    let connections: Vec<Connection> = read_table(IPV4_TABLE)?
        .map(|text| connected_sockets(&text).collect())
        .unwrap_or_default();
    let owners = socket_owners(
        &connections
            .iter()
            .map(|connection| connection.inode)
            .filter(|inode| *inode != 0)
            .collect(),
    );
    Ok(connections
        .into_iter()
        .map(|connection| TcpConnectionEntry {
            local: connection.local,
            remote: connection.remote,
            pid: owners.get(&connection.inode).copied(),
        })
        .collect())
}

fn read_table(table: &str) -> io::Result<Option<String>> {
    match fs::read_to_string(table) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn connected_sockets(table: &str) -> impl Iterator<Item = Connection> + '_ {
    table.lines().skip(1).filter_map(|line| {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if *fields.get(3)? == LISTEN_STATE {
            return None;
        }
        Some(Connection {
            local: ipv4_endpoint(fields.get(1)?)?,
            remote: ipv4_endpoint(fields.get(2)?)?,
            inode: fields.get(9)?.parse().ok()?,
        })
    })
}

fn ipv4_endpoint(field: &str) -> Option<SocketAddrV4> {
    let (address, port) = field.split_once(':')?;
    let address = u32::from_str_radix(address, 16).ok()?;
    let port = u16::from_str_radix(port, 16).ok()?;
    Some(SocketAddrV4::new(
        Ipv4Addr::from(address.to_ne_bytes()),
        port,
    ))
}

fn listening_sockets(table: &str) -> impl Iterator<Item = Socket> + '_ {
    table.lines().skip(1).filter_map(|line| {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if *fields.get(3)? != LISTEN_STATE {
            return None;
        }
        let (_, port) = fields.get(1)?.rsplit_once(':')?;
        Some(Socket {
            port: u16::from_str_radix(port, 16).ok()?,
            uid: fields.get(7)?.parse().ok()?,
            inode: fields.get(9)?.parse().ok()?,
        })
    })
}

fn socket_owners(inodes: &HashSet<u64>) -> HashMap<u64, u32> {
    let mut owners = HashMap::new();
    if inodes.is_empty() {
        return owners;
    }
    let Ok(processes) = fs::read_dir("/proc") else {
        return owners;
    };
    for process in processes.flatten() {
        let Some(pid) = process
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        let Ok(descriptors) = fs::read_dir(process.path().join("fd")) else {
            continue;
        };
        for descriptor in descriptors.flatten() {
            if let Some(inode) = fs::read_link(descriptor.path())
                .ok()
                .as_deref()
                .and_then(socket_inode)
                .filter(|inode| inodes.contains(inode))
            {
                owners.entry(inode).or_insert(pid);
            }
        }
    }
    owners
}

fn socket_inode(target: &Path) -> Option<u64> {
    target
        .to_str()?
        .strip_prefix("socket:[")?
        .strip_suffix(']')?
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 0100007F:A6EC 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1001        0 4242 1 0000000000000000 100 0 0 10 0
   1: 0100007F:A6EC 0100007F:D431 01 00000000:00000000 00:00000000 00000000  1000        0 4343 1 0000000000000000 20 4 30 10 -1
   2: 00000000000000000000000001000000:1F90 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 4444 1 0000000000000000 100 0 0 10 0
";

    #[test]
    fn only_listening_rows_are_read_with_their_port_owner_and_inode() {
        let sockets: Vec<(u16, u32, u64)> = listening_sockets(TABLE)
            .map(|socket| (socket.port, socket.uid, socket.inode))
            .collect();
        assert_eq!(sockets, [(42732, 1001, 4242), (8080, 1000, 4444)]);
    }

    #[test]
    fn only_connected_ipv4_rows_are_read_with_both_ends_and_inode() {
        let connections: Vec<(SocketAddrV4, SocketAddrV4, u64)> = connected_sockets(TABLE)
            .map(|connection| (connection.local, connection.remote, connection.inode))
            .collect();
        assert_eq!(
            connections,
            [(
                SocketAddrV4::new(Ipv4Addr::LOCALHOST, 42732),
                SocketAddrV4::new(Ipv4Addr::LOCALHOST, 54321),
                4343
            )]
        );
    }

    #[test]
    fn only_socket_links_name_an_inode() {
        assert_eq!(socket_inode(Path::new("socket:[4242]")), Some(4242));
        assert_eq!(socket_inode(Path::new("pipe:[4242]")), None);
        assert_eq!(socket_inode(Path::new("/dev/null")), None);
    }
}
