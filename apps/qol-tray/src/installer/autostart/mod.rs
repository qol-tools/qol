//! Single-owner facade for the per-OS autostart artifact (plist on macOS,
//! .desktop on Linux, .cmd on Windows). Both read and write live here so
//! callers do not have to choose between two modules.

mod platform;

use anyhow::Result;
use std::path::{Path, PathBuf};

pub fn read_target() -> Result<Option<PathBuf>> {
    platform::read_target()
}

pub fn write_target(binary: &Path) -> Result<()> {
    platform::write_target(binary)
}

pub fn autostart_path() -> Result<PathBuf> {
    platform::autostart_path()
}
