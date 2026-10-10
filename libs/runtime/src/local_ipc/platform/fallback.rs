use std::convert::Infallible;
use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::path::Path;
use std::time::Duration;

/// Neither type can be built here, so every method below is unreachable by
/// construction and the callers still type-check.
#[derive(Debug)]
pub struct LocalListener(Infallible);

#[derive(Debug)]
pub struct LocalStream(Infallible);

pub(super) const MAX_SOCKET_PATH_BYTES: usize = usize::MAX;

fn unsupported() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "local IPC is unavailable on this platform",
    )
}

impl LocalListener {
    pub fn incoming(&self) -> std::iter::Empty<io::Result<LocalStream>> {
        match self.0 {}
    }
}

impl LocalStream {
    pub fn connect(_path: impl AsRef<Path>) -> io::Result<LocalStream> {
        Err(unsupported())
    }

    pub fn try_clone(&self) -> io::Result<LocalStream> {
        match self.0 {}
    }

    pub fn set_read_timeout(&self, _timeout: Option<Duration>) -> io::Result<()> {
        match self.0 {}
    }

    pub fn set_write_timeout(&self, _timeout: Option<Duration>) -> io::Result<()> {
        match self.0 {}
    }

    pub fn shutdown(&self, _how: Shutdown) -> io::Result<()> {
        match self.0 {}
    }
}

impl Read for LocalStream {
    fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        match self.0 {}
    }
}

impl Read for &LocalStream {
    fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        match self.0 {}
    }
}

impl Write for LocalStream {
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        match self.0 {}
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.0 {}
    }
}

impl Write for &LocalStream {
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        match self.0 {}
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.0 {}
    }
}

pub(super) fn bind_listener(_path: &Path) -> io::Result<LocalListener> {
    Err(unsupported())
}

pub(super) fn authorize_peer(_stream: &LocalStream) -> io::Result<()> {
    Err(unsupported())
}
