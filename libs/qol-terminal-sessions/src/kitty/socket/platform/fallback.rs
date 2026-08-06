use std::path::Path;

pub(super) fn exchange(
    _path: &Path,
    _request: &[u8],
    _terminator: &[u8],
) -> std::io::Result<Vec<u8>> {
    Err(std::io::Error::other(
        "Kitty socket transport needs a Unix platform",
    ))
}
