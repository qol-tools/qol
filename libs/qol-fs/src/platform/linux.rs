use std::fs::File;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

pub(crate) fn set_private(file: &File) -> io::Result<()> {
    file.set_permissions(std::fs::Permissions::from_mode(0o600))
}

pub(crate) fn set_private_dir(path: &Path) -> io::Result<()> {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
}

pub(crate) fn sync_parent(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}
