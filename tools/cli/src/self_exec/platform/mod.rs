use std::ffi::OsString;
use std::path::Path;

use anyhow::Result;

#[cfg(not(any(unix, windows)))]
mod fallback;
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(not(any(unix, windows)))]
use fallback::Platform;
#[cfg(unix)]
use unix::Platform;
#[cfg(windows)]
use windows::Platform;

trait SelfExecPlatform {
    fn replace_process(&self, binary: &Path, args: &[OsString], tray_pid: u32) -> Result<()>;
}

pub(super) fn replace_process(binary: &Path, args: &[OsString], tray_pid: u32) -> Result<()> {
    Platform.replace_process(binary, args, tray_pid)
}
