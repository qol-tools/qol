use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use qol_conventions::DEFAULT_PORT;

pub struct InstanceLock;

pub fn claim() -> Option<InstanceLock> {
    let addr: SocketAddr = ([127, 0, 0, 1], DEFAULT_PORT).into();
    if TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_ok() {
        return None;
    }
    Some(InstanceLock)
}
