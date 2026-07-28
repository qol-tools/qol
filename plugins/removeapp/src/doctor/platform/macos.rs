use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;

use super::{DirectoryInspection, DirectoryState, PlatformInspection};

pub(crate) fn inspect() -> PlatformInspection {
    let home = absolute_home();
    let inventory_roots = inspect_paths(
        std::iter::once(PathBuf::from("/Applications"))
            .chain(home.as_ref().map(|home| home.join("Applications"))),
    );
    let trash = home
        .as_ref()
        .map(|home| inspect_directory(home.join(".Trash")));
    let trash_creation_anchor = home.map(inspect_directory);

    PlatformInspection {
        name: "macOS",
        supported: true,
        inventory_roots,
        trash,
        trash_creation_anchor,
    }
}

fn inspect_directory(path: impl Into<PathBuf>) -> DirectoryInspection {
    let path = path.into();
    let state = match std::fs::metadata(&path) {
        Ok(metadata) if metadata.is_dir() => DirectoryState::Directory,
        Ok(_) => DirectoryState::WrongType,
        Err(error) if error.kind() == ErrorKind::NotFound => DirectoryState::Missing,
        Err(error) => DirectoryState::Unreadable(error.kind()),
    };
    DirectoryInspection { path, state }
}

fn absolute_home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .filter(|home| home.is_absolute())
}

fn inspect_paths(paths: impl IntoIterator<Item = impl AsRef<Path>>) -> Vec<DirectoryInspection> {
    paths
        .into_iter()
        .map(|path| inspect_directory(path.as_ref().to_path_buf()))
        .collect()
}
