use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use super::{DirectoryInspection, DirectoryState};

pub(super) fn inspect_directory(path: impl Into<PathBuf>) -> DirectoryInspection {
    let path = path.into();
    let state = match std::fs::metadata(&path) {
        Ok(metadata) if metadata.is_dir() => DirectoryState::Directory,
        Ok(_) => DirectoryState::WrongType,
        Err(error) if error.kind() == ErrorKind::NotFound => DirectoryState::Missing,
        Err(error) => DirectoryState::Unreadable(error.kind()),
    };
    DirectoryInspection { path, state }
}

pub(super) fn inspect_paths(
    paths: impl IntoIterator<Item = impl AsRef<Path>>,
) -> Vec<DirectoryInspection> {
    paths
        .into_iter()
        .map(|path| inspect_directory(path.as_ref().to_path_buf()))
        .collect()
}
