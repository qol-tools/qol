use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::SortKey;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Folder,
    File,
    Link,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: OsString,
    pub display: String,
    pub path: PathBuf,
    pub id: Option<(u64, u64)>,
    pub kind: EntryKind,
    pub size: Option<u64>,
    pub hidden: bool,
    pub opens_as_folder: bool,
    pub sort_key: SortKey,
}

impl Entry {
    pub(super) fn read(item: &fs::DirEntry) -> io::Result<Self> {
        let name = item.file_name();
        let path = item.path();
        let metadata = match item.metadata() {
            Ok(metadata) => Some(metadata),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Err(error),
            Err(_) => None,
        };
        let (kind, size, opens_as_folder) = match &metadata {
            Some(metadata) => classify(&path, metadata),
            None => classify_unread(item),
        };
        let hidden = match &metadata {
            Some(metadata) => crate::places::is_hidden(&name, metadata),
            None => crate::places::is_hidden_item(item),
        };
        Ok(Self {
            display: display_name(&name),
            sort_key: SortKey::new(&name, opens_as_folder),
            id: metadata.as_ref().and_then(crate::places::file_id),
            name,
            path,
            kind,
            size,
            hidden,
            opens_as_folder,
        })
    }
}

pub fn display_name(name: &OsStr) -> String {
    qol_gpui::text_edit::marked(&name.to_string_lossy())
}

impl AsRef<Path> for Entry {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

fn classify(path: &Path, metadata: &fs::Metadata) -> (EntryKind, Option<u64>, bool) {
    if metadata.is_dir() {
        return (EntryKind::Folder, Some(0), true);
    }
    if !metadata.file_type().is_symlink() {
        return (EntryKind::File, Some(metadata.len()), false);
    }
    match fs::metadata(path) {
        Ok(target) if target.is_dir() => (EntryKind::Link, Some(0), true),
        Ok(target) => (EntryKind::Link, Some(target.len()), false),
        Err(_) => (EntryKind::Link, Some(0), false),
    }
}

fn classify_unread(item: &fs::DirEntry) -> (EntryKind, Option<u64>, bool) {
    match item.file_type() {
        Ok(kind) if kind.is_dir() => (EntryKind::Folder, None, true),
        Ok(kind) if kind.is_symlink() => (EntryKind::Link, None, false),
        _ => (EntryKind::File, None, false),
    }
}
