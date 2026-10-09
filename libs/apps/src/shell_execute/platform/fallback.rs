use std::ffi::OsStr;
use std::io;

pub(in crate::shell_execute) fn shell_execute(_target: &OsStr, _args: &[String]) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "shell execute is not supported on this platform",
    ))
}
