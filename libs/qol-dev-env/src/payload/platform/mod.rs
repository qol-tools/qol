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

trait PayloadPlatform {
    fn set_file_mode(&self, path: &Path, executable: bool) -> Result<()>;
    fn make_tree_read_only(&self, root: &Path) -> Result<()>;
    fn make_tree_writable(&self, root: &Path) -> Result<()>;
    #[cfg(test)]
    fn make_file_writable(&self, path: &Path) -> Result<()>;
}

pub(super) fn set_file_mode(path: &Path, executable: bool) -> Result<()> {
    Platform.set_file_mode(path, executable)
}

pub(super) fn make_tree_read_only(root: &Path) -> Result<()> {
    Platform.make_tree_read_only(root)
}

pub(super) fn make_tree_writable(root: &Path) -> Result<()> {
    Platform.make_tree_writable(root)
}

#[cfg(test)]
pub(super) fn make_file_writable(path: &Path) -> Result<()> {
    Platform.make_file_writable(path)
}
