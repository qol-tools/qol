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

pub(super) trait Connection: Read + Write + Send + Sync {
    fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()>;
    fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()>;
}

type Connected = Box<dyn Connection>;
type ConnectResult = io::Result<Connected>;

pub(super) fn connect(path: &Path) -> ConnectResult {
    active::connect(path)
}
