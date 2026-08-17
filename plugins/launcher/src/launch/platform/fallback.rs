use std::io;
use std::path::Path;

pub(crate) fn daemon_action_args(_path: &Path, _exec: &[String]) -> Option<(String, String)> {
    None
}

pub(crate) fn launch_app(_path: &Path, _exec: &[String]) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "application launching is unsupported on this platform",
    ))
}
