use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use super::super::unix::new_file;
use super::trash_dirs::{open_root, system_home};
use super::trash_info::contents;
use crate::ops::bin::{restore_exact, Bin, BinError, Held};
use crate::ops::print::Print;

const INFO_SUFFIX: &str = ".trashinfo";
const MAX_FILE_NAME: usize = 255;
const MAX_ITEM_NAME: usize = MAX_FILE_NAME - INFO_SUFFIX.len();
const MAX_EXTENSION: usize = 32;
const MAX_TRIES: u64 = 10_000;
const CONTINUATION_MASK: u8 = 0xC0;
const CONTINUATION: u8 = 0x80;

pub(super) struct Trasher {
    home: Option<PathBuf>,
}

impl Trasher {
    pub(super) fn new(home: Option<PathBuf>) -> Self {
        Self { home }
    }

    pub(super) fn system() -> Self {
        Self::new(system_home())
    }
}

impl Bin for Trasher {
    fn trash(&self, path: &Path) -> Result<Option<Held>, BinError> {
        let before = Print::read(path).map_err(|error| BinError::failed(&error))?;
        let original = real_place(path)?;
        let name = original
            .file_name()
            .ok_or_else(|| BinError::Failed("it has no name".into()))?;
        let root = open_root(self.home.as_deref(), &original).map_err(BinError::NoTrash)?;
        let text = contents(root.recorded(&original));
        for attempt in 1..=MAX_TRIES {
            let item_name = candidate(name.as_bytes(), attempt);
            let mut info_name = item_name.clone();
            info_name.push(INFO_SUFFIX);
            let info = root.info.join(info_name);
            let item = root.files.join(item_name);
            match reserve(&info, &text) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(BinError::failed(&error)),
            }
            match qol_fs::rename_noreplace(path, &item) {
                Ok(()) => {
                    return Ok(Some(Held {
                        item,
                        sidecar: Some(info),
                        before,
                    }))
                }
                Err(error) => {
                    forget(&info);
                    if error.kind() != io::ErrorKind::AlreadyExists {
                        return Err(BinError::failed(&error));
                    }
                }
            }
        }
        Err(BinError::Failed(
            "too many items with that name are already in the Trash".into(),
        ))
    }

    fn restore(&self, held: &Held, to: &Path) -> Result<(), BinError> {
        restore_exact(held, to)
    }
}

fn real_place(path: &Path) -> Result<PathBuf, BinError> {
    let absolute = std::path::absolute(path).map_err(|error| BinError::failed(&error))?;
    match (absolute.parent(), absolute.file_name()) {
        (Some(parent), Some(name)) => fs::canonicalize(parent)
            .map(|parent| parent.join(name))
            .map_err(|error| BinError::failed(&error)),
        _ => Ok(absolute),
    }
}

fn reserve(info: &Path, text: &str) -> io::Result<()> {
    let mut file = new_file(info)?;
    let written = file
        .write_all(text.as_bytes())
        .and_then(|()| file.sync_all());
    if written.is_err() {
        forget(info);
    }
    written
}

fn forget(info: &Path) {
    if let Err(error) = fs::remove_file(info) {
        if error.kind() != io::ErrorKind::NotFound {
            log::warn!("could not remove {}: {error}", info.display());
        }
    }
}

pub(super) fn candidate(name: &[u8], attempt: u64) -> OsString {
    let (stem, extension) = split(name);
    let suffix = if attempt <= 1 {
        String::new()
    } else {
        format!(" {attempt}")
    };
    let room = MAX_ITEM_NAME.saturating_sub(extension.len() + suffix.len());
    let mut out = shortened(stem, room).to_vec();
    out.extend_from_slice(suffix.as_bytes());
    out.extend_from_slice(extension);
    OsString::from_vec(out)
}

fn split(name: &[u8]) -> (&[u8], &[u8]) {
    match name.iter().rposition(|&byte| byte == b'.') {
        Some(dot) if dot > 0 && name.len() - dot <= MAX_EXTENSION => name.split_at(dot),
        _ => name.split_at(name.len()),
    }
}

fn shortened(stem: &[u8], room: usize) -> &[u8] {
    let mut end = stem.len().min(room);
    while end > 0 && end < stem.len() && stem[end] & CONTINUATION_MASK == CONTINUATION {
        end -= 1;
    }
    &stem[..end]
}
