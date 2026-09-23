use std::io::{self, Read, Write};
use std::path::Path;
use std::time::Duration;

#[cfg(not(unix))]
mod fallback;
#[cfg(unix)]
mod unix;

#[cfg(not(unix))]
use fallback as active;
#[cfg(unix)]
use unix as active;

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

pub(crate) fn connect(path: &Path) -> ConnectResult {
    active::connect(path)
}
