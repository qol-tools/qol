use super::Pacer;
use crate::platform::{BuildPlatform, Platform};
use std::fs;
use std::path::Path;
use std::time::SystemTime;

#[derive(Clone, Copy, Debug)]
pub(super) struct FileStat {
    pub(super) identity: Option<(u64, u64)>,
    pub(super) links: u64,
    pub(super) bytes: u64,
}

impl FileStat {
    pub(super) fn of(metadata: &fs::Metadata) -> Self {
        match Platform.file_identity(metadata) {
            Some(identity) => Self {
                identity: Some((identity.device, identity.inode)),
                links: identity.links,
                bytes: metadata.len(),
            },
            None => Self {
                identity: None,
                links: 1,
                bytes: metadata.len(),
            },
        }
    }
}

pub(super) fn read_entries(dir: &Path) -> impl Iterator<Item = fs::DirEntry> {
    fs::read_dir(dir).into_iter().flatten().flatten()
}

pub(super) fn collect_files(path: &Path, files: &mut Vec<FileStat>, pacer: &mut Pacer) {
    pacer.tick();
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return;
    };
    if !metadata.is_dir() {
        files.push(FileStat::of(&metadata));
        return;
    }
    for entry in read_entries(path) {
        collect_files(&entry.path(), files, pacer);
    }
}

pub(super) fn last_touched(path: &Path, pacer: &mut Pacer) -> SystemTime {
    pacer.tick();
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return SystemTime::UNIX_EPOCH;
    };
    let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
    if !metadata.is_dir() {
        return modified.max(metadata.accessed().unwrap_or(SystemTime::UNIX_EPOCH));
    }
    read_entries(path)
        .map(|entry| last_touched(&entry.path(), pacer))
        .fold(modified, SystemTime::max)
}
