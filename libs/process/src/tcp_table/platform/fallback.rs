use std::io;

use super::super::{TcpConnectionEntry, TcpListenerEntry};

pub fn tcp_listeners() -> io::Result<Vec<TcpListenerEntry>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "listing TCP listeners is unsupported on this platform",
    ))
}

pub fn tcp_connections() -> io::Result<Vec<TcpConnectionEntry>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "listing TCP connections is unsupported on this platform",
    ))
}
