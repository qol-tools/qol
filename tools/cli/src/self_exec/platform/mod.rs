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

#[cfg(not(any(unix, windows)))]
pub(crate) use fallback::{apply_prior_termios, capture_prior_termios, restore_resumed_tty};
#[cfg(unix)]
pub(crate) use unix::{apply_prior_termios, capture_prior_termios, restore_resumed_tty};
#[cfg(windows)]
pub(crate) use windows::{apply_prior_termios, capture_prior_termios, restore_resumed_tty};

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
