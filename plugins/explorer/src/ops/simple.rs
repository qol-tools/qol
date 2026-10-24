use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::{next_job_id, platform, Done, Failure, Verb};

#[cfg(test)]
#[path = "simple_tests.rs"]
mod tests;

const OS_ERROR_SUFFIX: &str = " (os error";
const SCRATCH_PREFIX: &str = ".";
const SCRATCH_SUFFIX: &str = ".qol-rename";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameProblem {
    Empty,
    Taken,
    Forbidden(String),
    TooLong,
    Io(String),
}

pub fn trash(paths: &[PathBuf]) -> Done {
    let mut trashed = Vec::new();
    let mut failures = Vec::new();
    for path in paths {
        match platform::trash_one(path) {
            Ok(()) => trashed.push(path.clone()),
            Err(reason) => failures.push(Failure {
                path: path.clone(),
                reason,
            }),
        }
    }
    let located = platform::trash_locations(&trashed);
    let placed: Vec<(PathBuf, PathBuf)> = trashed
        .into_iter()
        .map(|from| {
            let to = located
                .iter()
                .find(|(original, _)| *original == from)
                .map_or_else(|| from.clone(), |(_, to)| to.clone());
            (from, to)
        })
        .collect();
    Done {
        id: next_job_id(),
        verb: Verb::Move,
        landed: placed.clone(),
        placed,
        replaced: Vec::new(),
        kept_originals: Vec::new(),
        failures,
    }
}

pub fn restore(trashed: &[PathBuf]) -> Vec<Failure> {
    platform::restore_trashed(trashed)
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
    if differs_by_case_only(path, &target) {
        rename_case(path, &target)?;
    } else {
        if occupied(&target) {
            return Err(NameProblem::Taken);
        }
        fs::rename(path, &target).map_err(problem)?;
    }
    Ok(target)
}

pub fn new_folder(into: &Path, name: &str) -> Result<PathBuf, NameProblem> {
    if let Some(problem) = platform::name_problem(name) {
        return Err(problem);
    }
    let made = into.join(name);
    fs::create_dir(&made).map_err(problem)?;
    Ok(made)
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

fn rename_case(path: &Path, target: &Path) -> Result<(), NameProblem> {
    if !occupied(target) {
        return fs::rename(path, target).map_err(problem);
    }
    if has_sibling_named(target) {
        return Err(NameProblem::Taken);
    }
    let parent = target.parent().unwrap_or(Path::new(""));
    let old = path.file_name().unwrap_or_default();
    let mut scratch = OsString::from(SCRATCH_PREFIX);
    scratch.push(old);
    scratch.push(SCRATCH_SUFFIX);
    let scratch = free_name(parent, &scratch);
    fs::rename(path, &scratch).map_err(problem)?;
    if let Err(error) = fs::rename(&scratch, target) {
        let _ = fs::rename(&scratch, path);
        return Err(problem(error));
    }
    Ok(())
}

fn has_sibling_named(target: &Path) -> bool {
    let (Some(parent), Some(name)) = (target.parent(), target.file_name()) else {
        return false;
    };
    fs::read_dir(parent)
        .is_ok_and(|entries| entries.flatten().any(|entry| entry.file_name() == name))
}
