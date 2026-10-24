use std::path::{Path, PathBuf};
use std::time::SystemTime;

use trash::{os_limited, Error, TrashItem};

pub(super) type Listed = Result<Vec<TrashItem>, Error>;

use super::trash_reason::describe;
use crate::ops::Failure;

pub(super) type Locate = fn(&TrashItem) -> PathBuf;
pub(super) type OsError = fn(&Error) -> Option<String>;

pub(super) fn trash_one(path: &Path, os_error: OsError) -> Result<(), String> {
    trash::delete(path).map_err(|error| os_error(&error).unwrap_or_else(|| describe(&error)))
}

pub(super) fn locations(
    originals: &[PathBuf],
    listed: Listed,
    locate: Locate,
) -> Vec<(PathBuf, PathBuf)> {
    let Ok(items) = listed else {
        return Vec::new();
    };
    originals
        .iter()
        .filter_map(|from| {
            let item = newest(&items, &resolved(from), locate)?;
            Some((from.clone(), locate(item)))
        })
        .collect()
}

pub(super) fn restore(paths: &[PathBuf], listed: Listed, locate: Locate) -> Vec<Failure> {
    let items = match listed {
        Ok(items) => items,
        Err(error) => {
            let reason = describe(&error);
            return paths.iter().map(|path| failure(path, &reason)).collect();
        }
    };
    let mut failures = Vec::new();
    let mut chosen = Vec::new();
    for path in paths {
        let exact = items.iter().find(|item| locate(item) == *path);
        match exact.or_else(|| newest(&items, &resolved(path), locate)) {
            Some(item) => chosen.push((path.clone(), item.clone())),
            None => failures.push(failure(path, "it is no longer in the Trash")),
        }
    }
    failures.extend(put_back(chosen));
    failures
}

fn put_back(mut chosen: Vec<(PathBuf, TrashItem)>) -> Vec<Failure> {
    let mut failures = Vec::new();
    while !chosen.is_empty() {
        let before = chosen.len();
        match os_limited::restore_all(chosen.iter().map(|(_, item)| item.clone())) {
            Ok(()) => break,
            Err(Error::RestoreCollision {
                path,
                remaining_items,
            }) => {
                failures.push(failure(
                    &path,
                    "something with that name is already in its old folder",
                ));
                chosen.retain(|(_, item)| {
                    item.original_path() != path
                        && remaining_items.iter().any(|left| left.id == item.id)
                });
            }
            Err(Error::RestoreTwins { path, .. }) => {
                let mut first = true;
                chosen.retain(|(trashed, item)| {
                    if item.original_path() != path {
                        return true;
                    }
                    if std::mem::take(&mut first) {
                        return true;
                    }
                    failures.push(failure(
                        trashed,
                        "another item from the same place is being put back first",
                    ));
                    false
                });
            }
            Err(error) => {
                let reason = describe(&error);
                failures.extend(chosen.iter().map(|(path, _)| failure(path, &reason)));
                break;
            }
        }
        if chosen.len() == before {
            break;
        }
    }
    failures
}

fn newest<'a>(items: &'a [TrashItem], original: &Path, locate: Locate) -> Option<&'a TrashItem> {
    items
        .iter()
        .filter(|item| item.original_path() == original)
        .max_by_key(|item| (item.time_deleted, modified(&locate(item))))
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::symlink_metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
}

fn resolved(path: &Path) -> PathBuf {
    let parent = path.parent().and_then(|parent| parent.canonicalize().ok());
    match (parent, path.file_name()) {
        (Some(parent), Some(name)) => parent.join(name),
        _ => path.to_path_buf(),
    }
}

fn failure(path: &Path, reason: &str) -> Failure {
    Failure {
        path: path.to_path_buf(),
        reason: reason.to_string(),
    }
}
