#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
pub(crate) use unix::{console_probe, console_send, system_backends};
#[cfg(windows)]
pub(crate) use windows::{console_probe, console_send, system_backends};
