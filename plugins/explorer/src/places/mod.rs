mod platform;
#[cfg(test)]
mod tests;

use std::ffi::OsStr;
use std::fs::{DirEntry, FileType, Metadata};
use std::io;
use std::path::{Path, PathBuf};

use crate::listing::EntryKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaceKind {
    Home,
    Desktop,
    Documents,
    Downloads,
    Music,
    Pictures,
    Videos,
}

impl PlaceKind {
    pub const USER_FOLDERS: [Self; 6] = [
        Self::Desktop,
        Self::Documents,
        Self::Downloads,
        Self::Music,
        Self::Pictures,
        Self::Videos,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::Desktop => "Desktop",
            Self::Documents => "Documents",
            Self::Downloads => "Downloads",
            Self::Music => "Music",
            Self::Pictures => "Pictures",
            Self::Videos => "Videos",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub kind: PlaceKind,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    ThisComputer,
    Drive,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub kind: DeviceKind,
    pub label: String,
    pub path: PathBuf,
    pub free_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mount {
    pub label: String,
    pub path: PathBuf,
}

const THIS_COMPUTER: &str = "This computer";

pub fn places() -> Vec<Place> {
    let Some(home) = dirs::home_dir() else {
        log::warn!("the home folder is not known, so no places are listed");
        return Vec::new();
    };
    places_in(&home)
}

pub fn places_in(home: &Path) -> Vec<Place> {
    let known = platform::user_folders(home);
    let mut places = vec![Place {
        kind: PlaceKind::Home,
        path: home.to_path_buf(),
    }];
    places.extend(
        PlaceKind::USER_FOLDERS
            .into_iter()
            .map(|kind| Place {
                kind,
                path: user_folder(kind, &known, home),
            })
            .filter(|place| place.path != home && place.path.is_dir()),
    );
    places
}

pub fn devices() -> Vec<Device> {
    let root = platform::system_root();
    let mut devices = vec![device(
        DeviceKind::ThisComputer,
        THIS_COMPUTER.to_string(),
        root.clone(),
    )];
    devices.extend(
        platform::mounts()
            .into_iter()
            .filter(|mount| mount.path != root)
            .map(|mount| device(DeviceKind::Drive, mount.label, mount.path)),
    );
    devices
}

pub fn is_hidden(name: &OsStr, metadata: &Metadata) -> bool {
    platform::is_hidden(name, metadata)
}

pub fn is_executable(metadata: &Metadata) -> bool {
    platform::is_executable(metadata)
}

pub fn is_hidden_item(item: &DirEntry) -> bool {
    platform::is_hidden_item(item)
}

pub fn file_id(metadata: &Metadata) -> Option<(u64, u64)> {
    platform::file_id(metadata)
}

pub fn special_kind(file_type: &FileType) -> Option<EntryKind> {
    platform::special_kind(file_type)
}

pub fn is_pseudo_fs(folder: &Path) -> bool {
    platform::is_pseudo_fs(folder)
}

pub fn is_link_loop(error: &io::Error) -> bool {
    platform::is_link_loop(error)
}

fn user_folder(kind: PlaceKind, known: &[(PlaceKind, PathBuf)], home: &Path) -> PathBuf {
    known
        .iter()
        .find(|(known_kind, _)| *known_kind == kind)
        .map(|(_, path)| path.clone())
        .unwrap_or_else(|| home.join(kind.label()))
}

fn device(kind: DeviceKind, label: String, path: PathBuf) -> Device {
    let free_bytes = match qol_platform::disk_space(&path) {
        Ok(space) => Some(space.available),
        Err(error) => {
            log::debug!("no free space for {}: {error}", path.display());
            None
        }
    };
    Device {
        kind,
        label,
        path,
        free_bytes,
    }
}
