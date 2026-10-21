mod mountinfo;
#[cfg(test)]
mod tests;
mod user_dirs;

use std::ffi::{CString, OsStr};
use std::fs::{self, DirEntry, FileType, Metadata};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::{Path, PathBuf};

use crate::listing::EntryKind;
use crate::places::{Mount, PlaceKind};

const MOUNTINFO: &str = "/proc/self/mountinfo";
const USER_DIRS: &str = ".config/user-dirs.dirs";

pub(crate) fn user_folders(home: &Path) -> Vec<(PlaceKind, PathBuf)> {
    let file = home.join(USER_DIRS);
    match fs::read_to_string(&file) {
        Ok(contents) => user_dirs::parse(&contents, home),
        Err(error) => {
            log::debug!("no user folders from {}: {error}", file.display());
            Vec::new()
        }
    }
}

pub(crate) fn system_root() -> PathBuf {
    PathBuf::from("/")
}

pub(crate) fn mounts() -> Vec<Mount> {
    match fs::read_to_string(MOUNTINFO) {
        Ok(contents) => mountinfo::parse(&contents),
        Err(error) => {
            log::warn!("could not read {MOUNTINFO}: {error}");
            Vec::new()
        }
    }
}

pub(crate) fn is_executable(metadata: &Metadata) -> bool {
    metadata.mode() & 0o111 != 0
}

pub(crate) fn is_hidden(name: &OsStr, _metadata: &Metadata) -> bool {
    name.as_encoded_bytes().starts_with(b".")
}

pub(crate) fn is_hidden_item(item: &DirEntry) -> bool {
    item.file_name().as_encoded_bytes().starts_with(b".")
}

pub(crate) fn file_id(metadata: &Metadata) -> Option<(u64, u64)> {
    Some((metadata.dev(), metadata.ino()))
}

pub(crate) fn special_kind(file_type: &FileType) -> Option<EntryKind> {
    if file_type.is_fifo() {
        Some(EntryKind::Pipe)
    } else if file_type.is_socket() {
        Some(EntryKind::Socket)
    } else if file_type.is_block_device() || file_type.is_char_device() {
        Some(EntryKind::Device)
    } else {
        None
    }
}

pub(crate) fn is_pseudo_fs(folder: &Path) -> bool {
    let Ok(path) = CString::new(folder.as_os_str().as_bytes()) else {
        return false;
    };
    let mut filesystem: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(path.as_ptr(), &mut filesystem) } != 0 {
        return false;
    }
    matches!(
        filesystem.f_type,
        libc::PROC_SUPER_MAGIC
            | libc::SYSFS_MAGIC
            | libc::DEBUGFS_MAGIC
            | libc::TRACEFS_MAGIC
            | libc::SECURITYFS_MAGIC
            | libc::CGROUP_SUPER_MAGIC
            | libc::CGROUP2_SUPER_MAGIC
    )
}

pub(crate) fn is_link_loop(error: &io::Error) -> bool {
    error.raw_os_error() == Some(libc::ELOOP)
}
