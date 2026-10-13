use std::io;
use std::net::{IpAddr, Ipv4Addr};
use std::process::Command;

use super::super::{TcpConnectionEntry, TcpListenerEntry};

pub fn tcp_listeners() -> io::Result<Vec<TcpListenerEntry>> {
    Ok(parse_listeners(&lsof("-sTCP:LISTEN")?))
}

pub fn tcp_connections() -> io::Result<Vec<TcpConnectionEntry>> {
    Ok(parse_connections(&lsof("-sTCP:^LISTEN")?))
}

fn lsof(state: &str) -> io::Result<String> {
    let output = Command::new("/usr/sbin/lsof")
        .args(["-nP", "-iTCP", state, "-Fpn"])
        .output()?;
    if !output.status.success() && !output.stdout.is_empty() {
        return Err(io::Error::other(format!(
            "lsof failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn named_sockets(text: &str) -> impl Iterator<Item = (Option<u32>, &str)> {
    let mut pid = None;
    text.lines().filter_map(move |line| {
        if let Some(value) = line.strip_prefix('p') {
            pid = value.trim().parse::<u32>().ok();
            return None;
        }
        line.strip_prefix('n').map(|name| (pid, name))
    })
}

fn parse_listeners(text: &str) -> Vec<TcpListenerEntry> {
    named_sockets(text)
        .filter_map(|(pid, name)| {
            let (host, port) = name.rsplit_once(':')?;
            Some(TcpListenerEntry {
                address: listener_address(host)?,
                port: port.parse().ok()?,
                pid,
                uid: None,
            })
        })
        .collect()
}

fn listener_address(host: &str) -> Option<IpAddr> {
    if host == "*" {
        return Some(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
    }
    let host = host
        .strip_prefix('[')
        .and_then(|inner| inner.strip_suffix(']'))
        .unwrap_or(host);
    let host = host.split_once('%').map_or(host, |(address, _)| address);
    host.parse().ok()
}

fn parse_connections(text: &str) -> Vec<TcpConnectionEntry> {
    named_sockets(text)
        .filter_map(|(pid, name)| {
            let (local, remote) = name.split_once("->")?;
            Some(TcpConnectionEntry {
                local: local.parse().ok()?,
                remote: remote.parse().ok()?,
                pid,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv6Addr, SocketAddrV4};

    #[test]
    fn each_listening_name_is_read_under_the_process_above_it() {
        let parsed = parse_listeners(
            "p501\nf7\nn127.0.0.1:43210\nf9\nn[::1]:43210\np88\nf4\nn*:8080\nf5\nn[fe80::1%lo0]:5353\n",
        );
        let rows: Vec<(IpAddr, u16, Option<u32>)> = parsed
            .iter()
            .map(|entry| (entry.address, entry.port, entry.pid))
            .collect();
        assert_eq!(
            rows,
            [
                (Ipv4Addr::LOCALHOST.into(), 43210, Some(501)),
                (Ipv6Addr::LOCALHOST.into(), 43210, Some(501)),
                (Ipv4Addr::UNSPECIFIED.into(), 8080, Some(88)),
                (
                    Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1).into(),
                    5353,
                    Some(88)
                ),
            ]
        );
    }

    #[test]
    fn each_ipv4_connection_is_read_with_both_ends_under_its_process() {
        let parsed = parse_connections(
            "p501\nf7\nn127.0.0.1:50000->127.0.0.1:43210\nf9\nn[::1]:50001->[::1]:43210\np88\nf4\nn10.0.0.2:1234->93.184.216.34:443\nf5\nn*:5353\n",
        );
        let at = |a, b, c, d, port| SocketAddrV4::new(Ipv4Addr::new(a, b, c, d), port);
        assert_eq!(
            parsed,
            [
                TcpConnectionEntry {
                    local: at(127, 0, 0, 1, 50000),
                    remote: at(127, 0, 0, 1, 43210),
                    pid: Some(501),
                },
                TcpConnectionEntry {
                    local: at(10, 0, 0, 2, 1234),
                    remote: at(93, 184, 216, 34, 443),
                    pid: Some(88),
                },
            ]
        );
    }
}
