use std::ffi::OsStr;
use std::fs::{self, Metadata};
use std::os::macos::fs::MetadataExt;
use std::path::{Path, PathBuf};

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

pub(crate) fn is_hidden(name: &OsStr, metadata: &Metadata) -> bool {
    name.as_encoded_bytes().starts_with(b".") || metadata.st_flags() & libc::UF_HIDDEN != 0
}
