use std::ffi::OsStr;
use std::fs::Metadata;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};

use windows_sys::Win32::Storage::FileSystem::{
    GetDriveTypeW, GetLogicalDrives, FILE_ATTRIBUTE_HIDDEN,
};
use windows_sys::Win32::System::WindowsProgramming::{DRIVE_FIXED, DRIVE_REMOVABLE};

use crate::places::{Mount, PlaceKind};

const SYSTEM_DRIVE: &str = "SystemDrive";
const DEFAULT_SYSTEM_DRIVE: &str = "C:";
const DRIVE_LETTERS: u8 = 26;

pub(crate) fn user_folders(_home: &Path) -> Vec<(PlaceKind, PathBuf)> {
    [
        (PlaceKind::Desktop, dirs::desktop_dir()),
        (PlaceKind::Documents, dirs::document_dir()),
        (PlaceKind::Downloads, dirs::download_dir()),
        (PlaceKind::Music, dirs::audio_dir()),
        (PlaceKind::Pictures, dirs::picture_dir()),
        (PlaceKind::Videos, dirs::video_dir()),
    ]
    .into_iter()
    .filter_map(|(kind, path)| Some((kind, path?)))
    .collect()
}

pub(crate) fn system_root() -> PathBuf {
    let drive = std::env::var(SYSTEM_DRIVE).unwrap_or_else(|_| DEFAULT_SYSTEM_DRIVE.into());
    drive_root(&drive)
}

pub(crate) fn mounts() -> Vec<Mount> {
    // SAFETY: GetLogicalDrives takes no arguments and only returns a bitmask.
    let letters = unsafe { GetLogicalDrives() };
    (0..DRIVE_LETTERS)
        .filter(|bit| letters & (1u32 << bit) != 0)
        .filter_map(|bit| drive(char::from(b'A' + bit)))
        .collect()
}

pub(crate) fn is_hidden(_name: &OsStr, metadata: &Metadata) -> bool {
    metadata.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0
}

fn drive(letter: char) -> Option<Mount> {
    let label = format!("{letter}:");
    let path = drive_root(&label);
    let wide = qol_platform::native::wide::wide_nul(path.as_os_str());
    // SAFETY: `wide` is a NUL-terminated UTF-16 path that outlives the call.
    let kind = unsafe { GetDriveTypeW(wide.as_ptr()) };
    matches!(kind, DRIVE_FIXED | DRIVE_REMOVABLE).then_some(Mount { label, path })
}

fn drive_root(drive: &str) -> PathBuf {
    PathBuf::from(format!("{drive}\\"))
}
