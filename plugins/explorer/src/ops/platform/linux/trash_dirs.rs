use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use trash::{Error, TrashItem};

use super::in_trash;

const INFO: &str = "info";
const INFO_SUFFIX: &str = ".trashinfo";
const PATH_KEY: &str = "Path=";
const SHARED_TRASH: &str = ".Trash";
const HOME_TRASH: &str = "Trash";
const STICKY: u32 = 0o1000;

pub(super) fn items_near(paths: &[PathBuf]) -> Result<Vec<TrashItem>, Error> {
    let roots: BTreeSet<PathBuf> = paths
        .iter()
        .flat_map(|path| roots_for(path))
        .filter(|root| trusted(root))
        .collect();
    Ok(roots.iter().flat_map(|root| read_root(root)).collect())
}

fn roots_for(path: &Path) -> Vec<PathBuf> {
    if in_trash(path) {
        return path
            .parent()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .into_iter()
            .collect();
    }
    let Some(device) = device_of(path) else {
        return Vec::new();
    };
    let mut roots = Vec::new();
    if let Some(home) = data_home().map(|data| data.join(HOME_TRASH)) {
        if device_of(&home) == Some(device) {
            roots.push(home);
        }
    }
    if let Some(top) = topdir(path, device) {
        let uid = euid();
        roots.push(top.join(SHARED_TRASH).join(uid.to_string()));
        roots.push(top.join(format!("{SHARED_TRASH}-{uid}")));
    }
    roots
}

fn data_home() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| dirs::home_dir().map(|home| home.join(".local/share")))
}

fn euid() -> u32 {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

fn trusted(root: &Path) -> bool {
    let owned = fs::symlink_metadata(root).is_ok_and(|meta| meta.is_dir() && meta.uid() == euid());
    let shared = root
        .parent()
        .filter(|parent| parent.file_name() == Some(SHARED_TRASH.as_ref()));
    owned
        && shared.is_none_or(|parent| {
            fs::symlink_metadata(parent)
                .is_ok_and(|meta| meta.is_dir() && meta.mode() & STICKY != 0)
        })
}

fn device_of(path: &Path) -> Option<u64> {
    path.ancestors()
        .find_map(|ancestor| fs::symlink_metadata(ancestor).ok())
        .map(|metadata| metadata.dev())
}

fn topdir(path: &Path, device: u64) -> Option<PathBuf> {
    path.ancestors()
        .filter(|ancestor| fs::symlink_metadata(ancestor).is_ok_and(|meta| meta.dev() == device))
        .last()
        .map(Path::to_path_buf)
}

fn read_root(root: &Path) -> Vec<TrashItem> {
    let Ok(entries) = fs::read_dir(root.join(INFO)) else {
        return Vec::new();
    };
    let base = if root.parent().and_then(Path::file_name) == Some(SHARED_TRASH.as_ref()) {
        root.parent().and_then(Path::parent)
    } else {
        root.parent()
    };
    entries
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(INFO_SUFFIX))
        .filter_map(|entry| item(&entry.path(), base))
        .collect()
}

fn item(info: &Path, base: Option<&Path>) -> Option<TrashItem> {
    let meta = fs::symlink_metadata(info).ok()?;
    if !meta.is_file() || meta.uid() != euid() {
        return None;
    }
    let text = fs::read_to_string(info).ok()?;
    let encoded = text.lines().find_map(|line| line.strip_prefix(PATH_KEY))?;
    let decoded = PathBuf::from(OsString::from_vec(percent_decoded(encoded.trim())));
    if decoded
        .components()
        .any(|part| part == Component::ParentDir)
    {
        return None;
    }
    let original = if decoded.is_absolute() {
        decoded
    } else {
        base?.join(decoded)
    };
    let deleted = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |since| since.as_secs() as i64);
    Some(TrashItem {
        id: info.as_os_str().to_os_string(),
        name: original.file_name()?.to_os_string(),
        original_parent: original.parent()?.to_path_buf(),
        time_deleted: deleted,
    })
}

fn percent_decoded(text: &str) -> Vec<u8> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let hex = bytes
            .get(at + 1..at + 3)
            .and_then(|pair| std::str::from_utf8(pair).ok())
            .and_then(|pair| u8::from_str_radix(pair, 16).ok());
        match (bytes[at], hex) {
            (b'%', Some(byte)) => {
                out.push(byte);
                at += 3;
            }
            (byte, _) => {
                out.push(byte);
                at += 1;
            }
        }
    }
    out
}
