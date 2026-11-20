use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use super::super::unix::{euid, new_dir, STICKY};
use crate::ops::simple::plain;

const FILES: &str = "files";
const INFO: &str = "info";
const SHARED_TRASH: &str = ".Trash";
const HOME_TRASH: &str = "Trash";

pub(super) struct Root {
    pub(super) files: PathBuf,
    pub(super) info: PathBuf,
    pub(super) top: Option<PathBuf>,
}

impl Root {
    pub(super) fn recorded<'a>(&self, original: &'a Path) -> &'a Path {
        self.top
            .as_deref()
            .and_then(|top| original.strip_prefix(top).ok())
            .unwrap_or(original)
    }
}

pub(super) fn system_home() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| dirs::home_dir().map(|home| home.join(".local/share")))
        .map(|data| data.join(HOME_TRASH))
}

pub(super) fn open_root(home: Option<&Path>, path: &Path) -> Result<Root, String> {
    let (root, top) = root_for(home, path)?;
    prepare(root, top)
}

fn root_for(home: Option<&Path>, path: &Path) -> Result<(PathBuf, Option<PathBuf>), String> {
    let device = device_of(path).ok_or_else(|| "its drive could not be found".to_string())?;
    if let Some(home) = home.filter(|home| device_of(home) == Some(device)) {
        return Ok((home.to_path_buf(), None));
    }
    let top = topdir(path, device)
        .filter(|top| top != path)
        .ok_or_else(|| "the top of a drive cannot go to the Trash".to_string())?;
    Ok((shared_root(&top), Some(top)))
}

pub(super) fn shared_root(top: &Path) -> PathBuf {
    let uid = euid();
    let shared = top.join(SHARED_TRASH);
    let usable =
        fs::symlink_metadata(&shared).is_ok_and(|meta| meta.is_dir() && meta.mode() & STICKY != 0);
    if usable {
        shared.join(uid.to_string())
    } else {
        top.join(format!("{SHARED_TRASH}-{uid}"))
    }
}

fn prepare(root: PathBuf, top: Option<PathBuf>) -> Result<Root, String> {
    if let Some(parent) = root.parent() {
        fs::create_dir_all(parent).map_err(|error| plain(&error))?;
    }
    let files = root.join(FILES);
    let info = root.join(INFO);
    for folder in [&root, &files, &info] {
        made(folder)?;
    }
    Ok(Root { files, info, top })
}

fn made(folder: &Path) -> Result<(), String> {
    match new_dir(folder) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(plain(&error)),
    }
    let meta = fs::symlink_metadata(folder).map_err(|error| plain(&error))?;
    if meta.is_dir() && meta.uid() == euid() {
        Ok(())
    } else {
        Err("its Trash folder is not yours".into())
    }
}

fn device_of(path: &Path) -> Option<u64> {
    path.ancestors()
        .find_map(|ancestor| fs::symlink_metadata(ancestor).ok())
        .map(|metadata| metadata.dev())
}

fn topdir(path: &Path, device: u64) -> Option<PathBuf> {
    path.ancestors()
        .take_while(|ancestor| {
            fs::symlink_metadata(ancestor).is_ok_and(|meta| meta.dev() == device)
        })
        .last()
        .map(Path::to_path_buf)
}
