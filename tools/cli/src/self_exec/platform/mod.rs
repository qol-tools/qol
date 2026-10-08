use std::ffi::OsString;
use std::path::Path;

use anyhow::Result;

#[cfg(not(any(unix, windows)))]
mod fallback;
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
mod termios;
#[cfg(unix)]
pub(crate) use termios::{apply_prior_termios, capture_prior_termios, restore_resumed_tty};

#[cfg(not(unix))]
pub(crate) fn apply_prior_termios() {}
#[cfg(not(unix))]
pub(crate) fn capture_prior_termios() {}
#[cfg(not(unix))]
pub(crate) fn restore_resumed_tty() {}

#[cfg(not(any(unix, windows)))]
use fallback::Platform;
#[cfg(unix)]
use unix::Platform;
#[cfg(windows)]
use windows::Platform;

trait SelfExecPlatform {
    fn replace_process(&self, binary: &Path, args: &[OsString], env: (&str, &str)) -> Result<()>;
}

pub(super) fn replace_process(binary: &Path, args: &[OsString], env: (&str, &str)) -> Result<()> {
    Platform.replace_process(binary, args, env)
}
