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
    BrokenLink,
    Pipe,
    Socket,
    Device,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Size {
    Bytes(u64),
    Blank,
    Unreadable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: OsString,
    pub display: String,
    pub path: PathBuf,
    pub id: Option<(u64, u64)>,
    pub kind: EntryKind,
    pub size: Size,
    pub hidden: bool,
    pub opens_as_folder: bool,
    pub sort_key: SortKey,
}

impl Entry {
    pub(super) fn read(item: &fs::DirEntry, real_sizes: bool) -> io::Result<Self> {
        let name = item.file_name();
        let path = item.path();
        let metadata = match item.metadata() {
            Ok(metadata) => Some(metadata),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Err(error),
            Err(_) => None,
        };
        let (kind, size, opens_as_folder) = match &metadata {
            Some(metadata) => classify(&path, metadata, real_sizes),
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

fn classify(path: &Path, metadata: &fs::Metadata, real_sizes: bool) -> (EntryKind, Size, bool) {
    if metadata.is_dir() {
        return (EntryKind::Folder, Size::Blank, true);
    }
    if !metadata.file_type().is_symlink() {
        return match crate::places::special_kind(&metadata.file_type()) {
            Some(kind) => (kind, Size::Blank, false),
            None => (EntryKind::File, file_size(metadata, real_sizes), false),
        };
    }
    match fs::metadata(path) {
        Ok(target) if target.is_dir() => (EntryKind::Link, Size::Blank, true),
        Ok(target) => (EntryKind::Link, file_size(&target, real_sizes), false),
        Err(error) if target_is_gone(&error) => (EntryKind::BrokenLink, Size::Blank, false),
        Err(_) => (EntryKind::Link, Size::Unreadable, false),
    }
}

fn target_is_gone(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
    ) || crate::places::is_link_loop(error)
}

fn file_size(metadata: &fs::Metadata, real_sizes: bool) -> Size {
    if real_sizes && metadata.is_file() {
        Size::Bytes(metadata.len())
    } else {
        Size::Blank
    }
}

fn classify_unread(item: &fs::DirEntry) -> (EntryKind, Size, bool) {
    match item.file_type() {
        Ok(kind) if kind.is_dir() => (EntryKind::Folder, Size::Unreadable, true),
        Ok(kind) if kind.is_symlink() => (EntryKind::Link, Size::Unreadable, false),
        Ok(kind) => match crate::places::special_kind(&kind) {
            Some(special) => (special, Size::Blank, false),
            None => (EntryKind::File, Size::Unreadable, false),
        },
        Err(_) => (EntryKind::File, Size::Unreadable, false),
    }
}
