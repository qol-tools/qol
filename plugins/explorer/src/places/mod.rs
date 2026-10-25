mod platform;
#[cfg(test)]
mod tests;

use std::ffi::OsStr;
use std::fs::{DirEntry, FileType, Metadata};
use std::io;
use std::path::{is_separator, Path, PathBuf};

use crate::config::Pin;
use crate::listing::{display_name, EntryKind};
use crate::nav;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaceKind {
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
    pub pin: usize,
    pub name: String,
    pub path: PathBuf,
    pub home: bool,
    pub connected: bool,
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

pub fn places(pins: &[Pin]) -> Vec<Place> {
    let Some(home) = dirs::home_dir() else {
        log::warn!("the home folder is not known, so no places are listed");
        return Vec::new();
    };
    places_in(&home, pins)
}

pub fn places_in(home: &Path, pins: &[Pin]) -> Vec<Place> {
    let known = platform::user_folders(home);
    pins.iter()
        .enumerate()
        .filter_map(|(index, pin)| place(index, pin, home, &known))
        .collect()
}

pub fn pin_for(folder: &Path, home: &Path) -> Pin {
    pin_in(folder, home, &platform::user_folders(home))
}

fn pin_in(folder: &Path, home: &Path, known: &[(PlaceKind, PathBuf)]) -> Pin {
    let mut path = nav::path::display_with_home(folder, home);
    if let Some(kind) = usual_folder(&path) {
        if user_folder(kind, known, home) != folder {
            path = folder.to_string_lossy().into_owned();
        }
    }
    Pin {
        name: pin_name(folder, home),
        path,
    }
}

pub fn unpin(pins: &[Pin], places: &[Place], place: usize) -> Option<Vec<Pin>> {
    let pin = places.get(place)?.pin;
    if pin >= pins.len() {
        return None;
    }
    let mut pins = pins.to_vec();
    pins.remove(pin);
    Some(pins)
}

pub fn move_pin(pins: &[Pin], places: &[Place], place: usize, down: bool) -> Option<Vec<Pin>> {
    let neighbour = if down {
        place.checked_add(1)?
    } else {
        place.checked_sub(1)?
    };
    let (here, there) = (places.get(place)?.pin, places.get(neighbour)?.pin);
    if here >= pins.len() || there >= pins.len() {
        return None;
    }
    let mut pins = pins.to_vec();
    pins.swap(here, there);
    Some(pins)
}

pub fn toggle_pin(
    pins: &[Pin],
    places: &[Place],
    folder: &Path,
    pin: impl FnOnce() -> Pin,
) -> Option<Vec<Pin>> {
    let mut pins = pins.to_vec();
    match places.iter().find(|place| place.path == folder) {
        Some(place) if place.pin < pins.len() => {
            pins.remove(place.pin);
        }
        Some(_) => return None,
        None => pins.push(pin()),
    }
    Some(pins)
}

pub fn pin_at(pins: &[Pin], places: &[Place], slot: usize, added: Vec<Pin>) -> Option<Vec<Pin>> {
    let mut fresh: Vec<Pin> = Vec::new();
    for pin in added {
        let known = pins.iter().chain(&fresh).any(|kept| kept.path == pin.path);
        if !known {
            fresh.push(pin);
        }
    }
    if fresh.is_empty() {
        return None;
    }
    let at = match places.get(slot) {
        Some(place) => place.pin,
        None => places.last().map_or(pins.len(), |place| place.pin + 1),
    }
    .min(pins.len());
    let mut pins = pins.to_vec();
    pins.splice(at..at, fresh);
    Some(pins)
}

pub fn rename_pin(pins: &[Pin], pin: usize, name: &str) -> Option<Vec<Pin>> {
    let name = name.trim();
    if name.is_empty() || pins.get(pin)?.name == name {
        return None;
    }
    let mut pins = pins.to_vec();
    pins[pin].name = name.to_string();
    Some(pins)
}

pub fn device_label(devices: &[Device], path: &Path) -> Option<String> {
    devices
        .iter()
        .filter(|device| path.starts_with(&device.path))
        .max_by_key(|device| device.path.components().count())
        .map(|device| device.label.clone())
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

fn place(index: usize, pin: &Pin, home: &Path, known: &[(PlaceKind, PathBuf)]) -> Option<Place> {
    if pin.path.trim().is_empty() {
        return None;
    }
    let usual = usual_folder(&pin.path);
    let path = match usual {
        Some(kind) => user_folder(kind, known, home),
        None => nav::path::resolve(&pin.path, home, home),
    };
    let connected = path.is_dir();
    let is_home = path == home;
    if usual.is_some() && (is_home || !connected) {
        return None;
    }
    let name = if pin.name.trim().is_empty() {
        pin_name(&path, home)
    } else {
        pin.name.clone()
    };
    Some(Place {
        pin: index,
        name,
        path,
        home: is_home,
        connected,
    })
}

fn usual_folder(pin_path: &str) -> Option<PlaceKind> {
    let label = pin_path.strip_prefix('~')?.strip_prefix(is_separator)?;
    PlaceKind::USER_FOLDERS
        .into_iter()
        .find(|kind| kind.label() == label)
}

fn pin_name(folder: &Path, home: &Path) -> String {
    if folder == home {
        return "Home".to_string();
    }
    match folder.file_name() {
        Some(name) => display_name(name),
        None => folder.to_string_lossy().into_owned(),
    }
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
