use std::path::{Path, PathBuf};

#[cfg(not(unix))]
mod fallback;
#[cfg(unix)]
mod unix;

#[cfg(not(unix))]
use fallback as active;
#[cfg(unix)]
use unix as active;

pub(super) fn exchange(path: &Path, request: &[u8], terminator: &[u8]) -> std::io::Result<Vec<u8>> {
    active::exchange(path, request, terminator)
}

pub(super) fn discover_sibling_paths(current: &Path) -> Vec<PathBuf> {
    active::discover_sibling_paths(current)
}

pub(super) fn instance_id(path: &Path) -> Option<String> {
    active::instance_id(path)
}
