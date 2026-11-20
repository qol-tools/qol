use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::copy::remove_all;
use super::{platform, Bin, Failure, Trashing};

#[cfg(test)]
#[path = "simple_tests.rs"]
mod tests;

const OS_ERROR_SUFFIX: &str = " (os error";
const SCRATCH_PREFIX: &str = ".";
const SCRATCH_SUFFIX: &str = ".qol-rename";
const SCRATCH_VARIANTS: u32 = 99;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameProblem {
    Empty,
    Taken,
    Forbidden(String),
    TooLong,
    Io(String),
}

pub fn trash(bin: &dyn Bin, paths: &[PathBuf]) -> Trashing {
    let mut trashed = Vec::new();
    let mut failures = Vec::new();
    for path in paths {
        match bin.trash(path) {
            Ok(held) => trashed.push((path.clone(), held)),
            Err(error) => failures.push(Failure {
                path: path.clone(),
                reason: error.reason(),
            }),
        }
    }
    Trashing { trashed, failures }
}

pub fn rename(path: &Path, name: &str) -> Result<PathBuf, NameProblem> {
    if let Some(problem) = platform::name_problem(name) {
        return Err(problem);
    }
    let parent = path.parent().ok_or_else(|| {
        NameProblem::Io("it is the top of a drive and has no name to change".into())
    })?;
    let target = parent.join(name);
    if target == path {
        return Ok(target);
    }
    move_noreplace(path, &target).map_err(problem)?;
    Ok(target)
}

pub(in crate::ops) fn move_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    if from.parent() == to.parent() && differs_by_case_only(from, to) && occupied(to) {
        if has_sibling_named(to) {
            return Err(io::ErrorKind::AlreadyExists.into());
        }
        return move_through_scratch(from, to);
    }
    qol_fs::rename_noreplace(from, to)
}

pub fn new_folder(into: &Path, name: &str) -> Result<PathBuf, NameProblem> {
    if let Some(problem) = platform::name_problem(name) {
        return Err(problem);
    }
    let made = into.join(name);
    fs::create_dir(&made).map_err(problem)?;
    Ok(made)
}

pub fn new_file(into: &Path, name: &str) -> Result<PathBuf, NameProblem> {
    if let Some(problem) = platform::name_problem(name) {
        return Err(problem);
    }
    let made = into.join(name);
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&made)
        .map_err(problem)?;
    Ok(made)
}

pub fn delete(paths: &[PathBuf]) -> Vec<Failure> {
    paths
        .iter()
        .filter_map(|path| {
            remove_all(path).err().map(|error| Failure {
                path: path.clone(),
                reason: plain(&error),
            })
        })
        .collect()
}

pub fn copy_name(into: &Path, name: &OsStr, folder: bool) -> PathBuf {
    let (stem, extension) = if folder {
        (name.to_os_string(), None)
    } else {
        split_extension(name)
    };
    let mut number = 1u32;
    loop {
        let mut candidate = stem.clone();
        if number == 1 {
            candidate.push(" copy");
        } else {
            candidate.push(format!(" copy {number}"));
        }
        if let Some(extension) = &extension {
            candidate.push(".");
            candidate.push(extension);
        }
        let path = into.join(candidate);
        if !occupied(&path) {
            return path;
        }
        number += 1;
    }
}

pub fn free_name(into: &Path, name: &OsStr) -> PathBuf {
    let first = into.join(name);
    if !occupied(&first) {
        return first;
    }
    let folder = fs::symlink_metadata(&first).is_ok_and(|metadata| metadata.is_dir());
    let (stem, extension) = if folder {
        (name.to_os_string(), None)
    } else {
        split_extension(name)
    };
    let mut number = 2u32;
    loop {
        let mut candidate = stem.clone();
        candidate.push(format!(" ({number})"));
        if let Some(extension) = &extension {
            candidate.push(".");
            candidate.push(extension);
        }
        let path = into.join(candidate);
        if !occupied(&path) {
            return path;
        }
        number += 1;
    }
}

pub(in crate::ops) fn plain(error: &io::Error) -> String {
    let text = error.to_string();
    let text = text.split(OS_ERROR_SUFFIX).next().unwrap_or(&text);
    use io::ErrorKind::*;
    match error.kind() {
        PermissionDenied => "you do not have permission to change it".into(),
        NotFound => "it is no longer there".into(),
        AlreadyExists => "something with that name is already there".into(),
        StorageFull => "the drive is full".into(),
        ReadOnlyFilesystem => "the drive is read-only".into(),
        ResourceBusy | ExecutableFileBusy => "it is open in another app".into(),
        CrossesDevices => "it cannot move between these drives directly".into(),
        InvalidData => "the copy did not match the original".into(),
        Unsupported => "this drive cannot hold it".into(),
        _ => lowercase_first(text),
    }
}

fn lowercase_first(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn problem(error: io::Error) -> NameProblem {
    match error.kind() {
        io::ErrorKind::AlreadyExists => NameProblem::Taken,
        _ => NameProblem::Io(plain(&error)),
    }
}

fn occupied(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

fn split_extension(name: &OsStr) -> (OsString, Option<OsString>) {
    let path = Path::new(name);
    match (path.file_stem(), path.extension()) {
        (Some(stem), Some(extension)) => (stem.to_os_string(), Some(extension.to_os_string())),
        _ => (name.to_os_string(), None),
    }
}

fn differs_by_case_only(path: &Path, target: &Path) -> bool {
    let (Some(old), Some(new)) = (path.file_name(), target.file_name()) else {
        return false;
    };
    old != new && old.to_string_lossy().to_lowercase() == new.to_string_lossy().to_lowercase()
}

fn move_through_scratch(from: &Path, to: &Path) -> io::Result<()> {
    let parent = to.parent().unwrap_or(Path::new(""));
    let old = from.file_name().unwrap_or_default();
    let mut base = OsString::from(SCRATCH_PREFIX);
    base.push(old);
    base.push(SCRATCH_SUFFIX);
    let scratch = park(from, parent, &base)?;
    let Err(error) = qol_fs::rename_noreplace(&scratch, to) else {
        return Ok(());
    };
    if let Err(restore_error) = qol_fs::rename_noreplace(&scratch, from) {
        log::error!(
            "could not move {} back to {}: {restore_error}",
            scratch.display(),
            from.display()
        );
    }
    Err(error)
}

fn park(from: &Path, parent: &Path, base: &OsStr) -> io::Result<PathBuf> {
    for number in 1..=SCRATCH_VARIANTS {
        let mut name = base.to_os_string();
        if number > 1 {
            name.push(format!(" ({number})"));
        }
        let scratch = parent.join(name);
        match qol_fs::rename_noreplace(from, &scratch) {
            Ok(()) => return Ok(scratch),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::ErrorKind::AlreadyExists.into())
}

fn has_sibling_named(target: &Path) -> bool {
    let (Some(parent), Some(name)) = (target.parent(), target.file_name()) else {
        return false;
    };
    fs::read_dir(parent)
        .is_ok_and(|entries| entries.flatten().any(|entry| entry.file_name() == name))
}
