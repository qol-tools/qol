use std::io::{self, Read, Write};
use std::path::Path;
use std::time::Duration;

use crate::local_ipc::LocalStream;

#[cfg(not(any(unix, windows)))]
mod fallback;
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(not(any(unix, windows)))]
use fallback as active;
#[cfg(unix)]
use unix as active;
#[cfg(windows)]
use windows as active;

pub(super) use active::fallback_state_socket;

pub(crate) trait Connection: Read + Write + Send + Sync {
    fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()>;
    fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()>;
    /// A second handle to the same connection, so one side can read
    /// while the other shuts the socket down.
    fn try_clone(&self) -> io::Result<Box<dyn Connection>>;
    /// Close the connection's socket so a blocked reader observes EOF.
    /// Used by the event bus client to stop a subscription reader.
    fn shutdown(&self, how: std::net::Shutdown) -> io::Result<()>;
}

type Connected = Box<dyn Connection>;
type ConnectResult = io::Result<Connected>;

impl Connection for LocalStream {
    fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        LocalStream::set_read_timeout(self, timeout)
    }

    fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        LocalStream::set_write_timeout(self, timeout)
    }

    fn try_clone(&self) -> io::Result<Box<dyn Connection>> {
        Ok(Box::new(LocalStream::try_clone(self)?))
    }

    fn shutdown(&self, how: std::net::Shutdown) -> io::Result<()> {
        LocalStream::shutdown(self, how)
    }
}

pub(crate) fn connect(path: &Path) -> ConnectResult {
    active::connect(path)
}
