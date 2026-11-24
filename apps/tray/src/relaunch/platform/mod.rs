mod fallback_non_unix;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(unix)]
mod unix_other;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(all(
    not(unix),
    not(any(target_os = "linux", target_os = "macos", target_os = "windows"))
))]
use fallback_non_unix as imp;
#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(all(
    unix,
    not(any(target_os = "linux", target_os = "macos", target_os = "windows"))
))]
use unix_other as imp;
#[cfg(target_os = "windows")]
use windows as imp;

use std::ffi::OsString;
use std::path::Path;

trait ReplacePlatformOps {
    fn replace_process(binary: &Path, args: &[OsString]) -> String;
}

pub(super) fn replace_process(binary: &Path, args: &[OsString]) -> String {
    imp::Platform::replace_process(binary, args)
}

const _: fallback_non_unix::Platform = fallback_non_unix::Platform;
#[cfg(unix)]
const _: unix_other::Platform = unix_other::Platform;
