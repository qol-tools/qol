#[cfg(test)]
mod tests;
mod trash_dirs;

use std::path::{Path, PathBuf};

use trash::{Error, TrashItem};

use super::bin;
use crate::ops::simple::plain;
use crate::ops::Failure;

const TRASH_FILES: &str = "files";
const TRASH_DIR: &str = "Trash";
const TRASH_DIR_PREFIX: &str = ".Trash";

pub(crate) fn trash_one(path: &Path) -> Result<(), String> {
    bin::trash_one(path, os_error)
}

pub(crate) fn trash_locations(originals: &[PathBuf]) -> Vec<(PathBuf, PathBuf)> {
    bin::locations(originals, trash_dirs::items_near(originals), locate)
}

pub(crate) fn restore_trashed(paths: &[PathBuf]) -> Vec<Failure> {
    bin::restore(paths, trash_dirs::items_near(paths), locate)
}

pub(crate) fn in_trash(path: &Path) -> bool {
    let names: Vec<_> = path.components().map(|part| part.as_os_str()).collect();
    names.windows(2).any(|pair| {
        let folder = pair[0].to_string_lossy();
        pair[1] == TRASH_FILES && (folder == TRASH_DIR || folder.starts_with(TRASH_DIR_PREFIX))
    })
}

fn os_error(error: &Error) -> Option<String> {
    match error {
        Error::FileSystem { source, .. } => Some(plain(source)),
        _ => None,
    }
}

fn locate(item: &TrashItem) -> PathBuf {
    let info = Path::new(&item.id);
    let root = info.parent().and_then(Path::parent);
    match (root, info.file_stem()) {
        (Some(root), Some(stem)) => root.join(TRASH_FILES).join(stem),
        _ => info.to_path_buf(),
    }
}
