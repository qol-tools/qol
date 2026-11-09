use std::fs::File;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use super::rename_check::rename_if_absent;
use super::unix_call::{already_exists, path_cstring};

pub(crate) fn set_private(file: &File) -> io::Result<()> {
    file.set_permissions(std::fs::Permissions::from_mode(0o600))
}

pub(crate) fn set_private_dir(path: &Path) -> io::Result<()> {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
}

pub(crate) fn ensure_private_file(path: &Path, metadata: &std::fs::Metadata) -> io::Result<()> {
    if metadata.permissions().mode() & 0o077 == 0 {
        return Ok(());
    }
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

pub(crate) fn sync_parent(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

pub(crate) fn prepare_file_removal(_: &Path) -> io::Result<()> {
    Ok(())
}

pub(crate) fn set_mode(file: &File, mode: u32) -> io::Result<()> {
    file.set_permissions(std::fs::Permissions::from_mode(mode))
}

pub fn file_identity(_: &Path, metadata: &std::fs::Metadata) -> Option<crate::FileIdentity> {
    use std::os::unix::fs::MetadataExt;
    Some(crate::FileIdentity {
        device: metadata.dev(),
        index: u128::from(metadata.ino()),
        links: metadata.nlink(),
    })
}

pub fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    let from_c = path_cstring(from)?;
    let to_c = path_cstring(to)?;
    let status = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            from_c.as_ptr(),
            libc::AT_FDCWD,
            to_c.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if status == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    match error.raw_os_error() {
        Some(libc::EINVAL | libc::ENOSYS) => rename_if_absent(from, to),
        _ => Err(already_exists(error)),
    }
}
