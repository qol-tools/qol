use std::ffi::OsStr;
use std::fs::{self, DirEntry, FileType, Metadata};
use std::io;
use std::os::macos::fs::MetadataExt;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::{Path, PathBuf};

use crate::listing::EntryKind;
use crate::places::{Mount, PlaceKind};

const VOLUMES: &str = "/Volumes";

pub(crate) fn user_folders(_home: &Path) -> Vec<(PlaceKind, PathBuf)> {
    Vec::new()
}

pub(crate) fn system_root() -> PathBuf {
    PathBuf::from("/")
}

pub(crate) fn mounts() -> Vec<Mount> {
    let volumes = match fs::read_dir(VOLUMES) {
        Ok(volumes) => volumes,
        Err(error) => {
            log::warn!("could not read {VOLUMES}: {error}");
            return Vec::new();
        }
    };
    volumes
        .flatten()
        .filter(|volume| !volume.file_name().as_encoded_bytes().starts_with(b"."))
        .map(|volume| volume.path())
        .filter(|path| fs::canonicalize(path).is_ok_and(|real| real != system_root()))
        .map(|path| Mount {
            label: path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            path,
        })
        .collect()
}

pub(crate) fn is_executable(metadata: &Metadata) -> bool {
    metadata.mode() & 0o111 != 0
}

pub(crate) fn is_hidden(name: &OsStr, metadata: &Metadata) -> bool {
    name.as_encoded_bytes().starts_with(b".") || metadata.st_flags() & libc::UF_HIDDEN != 0
}

pub(crate) fn is_hidden_item(item: &DirEntry) -> bool {
    item.file_name().as_encoded_bytes().starts_with(b".")
        || item
            .metadata()
            .is_ok_and(|metadata| metadata.st_flags() & libc::UF_HIDDEN != 0)
}

pub(crate) fn file_id(metadata: &Metadata) -> Option<(u64, u64)> {
    Some((metadata.st_dev(), metadata.st_ino()))
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

pub(crate) fn is_pseudo_fs(_folder: &Path) -> bool {
    false
}

pub(crate) fn is_link_loop(error: &io::Error) -> bool {
    error.raw_os_error() == Some(libc::ELOOP)
}
