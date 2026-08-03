use std::io;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use super::{ConnectResult, Connection};

impl Connection for UnixStream {
    fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        UnixStream::set_read_timeout(self, timeout)
    }

    fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        UnixStream::set_write_timeout(self, timeout)
    }

    fn try_clone(&self) -> io::Result<Box<dyn Connection>> {
        Ok(Box::new(UnixStream::try_clone(self)?))
    }

    fn shutdown(&self, how: std::net::Shutdown) -> io::Result<()> {
        UnixStream::shutdown(self, how)
    }
}

pub(super) fn connect(path: &Path) -> ConnectResult {
    let stream = UnixStream::connect(path)?;
    crate::local_ipc::authorize_peer(&stream)?;
    Ok(Box::new(stream))
}
