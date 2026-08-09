use std::path::{Path, PathBuf};

pub(super) fn exchange(
    _path: &Path,
    _request: &[u8],
    _terminator: &[u8],
) -> std::io::Result<Vec<u8>> {
    Err(std::io::Error::other(
        "Kitty socket transport needs a Unix platform",
    ))
}

pub(super) fn discover_sibling_paths(current: &Path) -> Vec<PathBuf> {
    vec![current.to_owned()]
}

pub(super) fn discover_default_paths() -> Vec<PathBuf> {
    Vec::new()
}

pub(super) fn instance_id(_path: &Path) -> Option<String> {
    None
}
