use std::os::unix::fs::PermissionsExt;
use std::path::Path;

pub(super) fn is_executable(path: &Path) -> bool {
    std::fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}
