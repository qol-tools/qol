#[cfg(test)]
mod tests;

use std::fs;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::{symlink_dir, symlink_file};
use std::path::{Path, PathBuf};

use trash::{Error, TrashItem};
use windows_sys::Win32::Storage::FileSystem::{
    GetDiskFreeSpaceExW, GetVolumeInformationW, GetVolumePathNameW,
};

use super::bin;
use crate::ops::simple::plain;
use crate::ops::{Failure, NameProblem};

const MAX_NAME_UNITS: usize = 255;
const ROOT_BUFFER: usize = 261;
const RECYCLE_BIN: &str = "$Recycle.Bin";
const RESERVED_CHARS: &str = "<>:\"/\\|?*";
const RESERVED_NAMES: [&str; 4] = ["CON", "PRN", "AUX", "NUL"];
const NUMBERED_RESERVED: [&str; 2] = ["COM", "LPT"];

pub(crate) fn free_bytes(path: &Path) -> io::Result<u64> {
    let wide = wide(existing_folder(path).as_path());
    let mut available = 0u64;
    // SAFETY: `wide` is NUL-terminated and `available` outlives the call; the other outputs are optional.
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(available)
}

pub(crate) fn volume_of(path: &Path) -> io::Result<u64> {
    let wide = wide(path);
    let mut root = [0u16; ROOT_BUFFER];
    // SAFETY: `wide` is NUL-terminated and `root` is writable for its stated length.
    let ok = unsafe { GetVolumePathNameW(wide.as_ptr(), root.as_mut_ptr(), root.len() as u32) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut serial = 0u32;
    // SAFETY: `root` is NUL-terminated by the call above and `serial` outlives this call; the other outputs are optional.
    let ok = unsafe {
        GetVolumeInformationW(
            root.as_ptr(),
            std::ptr::null_mut(),
            0,
            &mut serial,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(u64::from(serial))
}

pub(crate) fn removable(path: &Path) -> Result<(), String> {
    fs::symlink_metadata(path)
        .map(|_| ())
        .map_err(|error| format!("cannot be reached, {}", plain(&error)))
}

fn existing_folder(path: &Path) -> PathBuf {
    path.ancestors()
        .find(|candidate| candidate.is_dir())
        .unwrap_or(path)
        .to_path_buf()
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

pub(crate) fn name_problem(name: &str) -> Option<NameProblem> {
    if name.trim().is_empty() {
        return Some(NameProblem::Empty);
    }
    if let Some(bad) = name.chars().find(|c| RESERVED_CHARS.contains(*c)) {
        return Some(NameProblem::Forbidden(format!(
            "Windows does not allow {bad} in names"
        )));
    }
    if name.chars().any(|c| c.is_control()) {
        return Some(NameProblem::Forbidden(
            "Windows does not allow control characters in names".into(),
        ));
    }
    if name.ends_with(' ') || name.ends_with('.') {
        return Some(NameProblem::Forbidden(
            "Windows does not allow a name to end in a space or a dot".into(),
        ));
    }
    if let Some(reserved) = reserved_device(name) {
        return Some(NameProblem::Forbidden(format!(
            "{reserved} is reserved on Windows"
        )));
    }
    if name.encode_utf16().count() > MAX_NAME_UNITS {
        return Some(NameProblem::TooLong);
    }
    None
}

fn reserved_device(name: &str) -> Option<String> {
    let stem = name.split('.').next().unwrap_or(name).trim_end();
    let upper = stem.to_uppercase();
    let plain_device = RESERVED_NAMES.contains(&upper.as_str());
    let numbered = NUMBERED_RESERVED.iter().any(|prefix| {
        upper
            .strip_prefix(prefix)
            .is_some_and(|digit| matches!(digit.as_bytes(), [b'1'..=b'9']))
    });
    (plain_device || numbered).then_some(upper)
}

pub(crate) fn copy_link(from: &Path, to: &Path) -> io::Result<()> {
    let target = fs::read_link(from)?;
    let folder = fs::metadata(from).map(|m| m.is_dir()).unwrap_or(false);
    if folder {
        symlink_dir(target, to)
    } else {
        symlink_file(target, to)
    }
}

pub(crate) fn trash_one(path: &Path) -> Result<(), String> {
    bin::trash_one(path, os_error)
}

pub(crate) fn trash_locations(originals: &[PathBuf]) -> Vec<(PathBuf, PathBuf)> {
    bin::locations(originals, trash::os_limited::list(), locate)
}

pub(crate) fn restore_trashed(paths: &[PathBuf]) -> Vec<Failure> {
    bin::restore(paths, trash::os_limited::list(), locate)
}

pub(crate) fn in_trash(path: &Path) -> bool {
    path.components()
        .any(|part| part.as_os_str().eq_ignore_ascii_case(RECYCLE_BIN))
}

fn os_error(_error: &Error) -> Option<String> {
    None
}

fn locate(item: &TrashItem) -> PathBuf {
    PathBuf::from(&item.id)
}
