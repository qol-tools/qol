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
    pub size: u64,
    pub hidden: bool,
    pub opens_as_folder: bool,
    pub sort_key: SortKey,
}

impl Entry {
    pub(super) fn read(item: &fs::DirEntry) -> io::Result<Self> {
        let name = item.file_name();
        let path = item.path();
        let metadata = item.metadata()?;
        let hidden = crate::places::is_hidden(&name, &metadata);
        let (kind, size, opens_as_folder) = classify(&path, &metadata);
        Ok(Self {
            display: display_name(&name),
            sort_key: SortKey::new(&name, opens_as_folder),
            id: crate::places::file_id(&metadata),
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
    name.to_string_lossy()
        .chars()
        .map(|letter| {
            if needs_mark(letter) {
                char::REPLACEMENT_CHARACTER
            } else {
                letter
            }
        })
        .collect()
}

pub fn needs_mark(letter: char) -> bool {
    letter.is_control()
        || matches!(
            letter,
            '\u{061C}'
                | '\u{200B}'
                | '\u{200E}'
                | '\u{200F}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2060}'
                | '\u{2066}'..='\u{2069}'
                | '\u{FEFF}'
        )
}

impl AsRef<Path> for Entry {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

fn classify(path: &Path, metadata: &fs::Metadata) -> (EntryKind, u64, bool) {
    if metadata.is_dir() {
        return (EntryKind::Folder, 0, true);
    }
    if !metadata.file_type().is_symlink() {
        return (EntryKind::File, metadata.len(), false);
    }
    match fs::metadata(path) {
        Ok(target) if target.is_dir() => (EntryKind::Link, 0, true),
        Ok(target) => (EntryKind::Link, target.len(), false),
        Err(_) => (EntryKind::Link, 0, false),
    }
}
