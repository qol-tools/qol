use std::io;
use std::sync::Arc;

use crate::kitty::KittyBackend;
use crate::{BackendId, TerminalBackend};

pub(crate) fn system_backends() -> Vec<Arc<dyn TerminalBackend>> {
    vec![Arc::new(KittyBackend::default())]
}

pub(crate) fn spawn_backend() -> &'static BackendId {
    crate::kitty::backend_id()
}

pub(crate) fn console_probe(_args: &[String]) -> io::Result<String> {
    Err(unsupported())
}

pub(crate) fn console_send(_args: &[String]) -> io::Result<String> {
    Err(unsupported())
}

pub(crate) fn console_launch(_args: &[String]) -> io::Result<String> {
    Err(unsupported())
}

pub(crate) fn console_title(_args: &[String]) -> io::Result<String> {
    Err(unsupported())
}

fn unsupported() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "console helpers drive Windows consoles and are unavailable on this platform",
    )
}
