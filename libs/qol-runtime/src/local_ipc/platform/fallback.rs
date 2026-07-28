use std::io;
use std::path::Path;

#[derive(Debug)]
pub struct LocalListener;

#[derive(Debug)]
pub struct LocalStream;

fn unsupported() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "local IPC is unavailable on this platform",
    )
}

pub(super) fn bind_listener(_path: &Path) -> io::Result<LocalListener> {
    Err(unsupported())
}

pub(super) fn authorize_peer(_stream: &LocalStream) -> io::Result<()> {
    Err(unsupported())
}
