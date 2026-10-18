use std::ffi::OsStr;
use std::fs::Metadata;
use std::path::{Path, PathBuf};

use crate::places::{Mount, PlaceKind};

pub(crate) fn user_folders(_home: &Path) -> Vec<(PlaceKind, PathBuf)> {
    Vec::new()
}

pub(crate) fn system_root() -> PathBuf {
    PathBuf::from("/")
}

pub(crate) fn mounts() -> Vec<Mount> {
    log::warn!("drives are not listed on this platform");
    Vec::new()
}

pub(crate) fn is_hidden(name: &OsStr, _metadata: &Metadata) -> bool {
    name.as_encoded_bytes().starts_with(b".")
}
