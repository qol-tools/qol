mod mountinfo;
#[cfg(test)]
mod tests;
mod user_dirs;

use std::ffi::OsStr;
use std::fs::{self, Metadata};
use std::path::{Path, PathBuf};

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

pub(crate) fn is_hidden(name: &OsStr, _metadata: &Metadata) -> bool {
    name.as_encoded_bytes().starts_with(b".")
}
