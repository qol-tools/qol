use std::ffi::OsString;
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
            display: name.to_string_lossy().into_owned(),
            sort_key: SortKey::new(&name, opens_as_folder),
            name,
            path,
            kind,
            size,
            hidden,
            opens_as_folder,
        })
    }
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
