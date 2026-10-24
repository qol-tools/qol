#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

use crate::ops::{Failure, NameProblem};

const NO_RESTORE: &str =
    "macOS does not let apps put things back from the Trash, use Put Back in Finder";

pub(crate) fn name_problem(name: &str) -> Option<NameProblem> {
    if name.contains(':') {
        return Some(NameProblem::Forbidden(
            "macOS does not allow : in names".into(),
        ));
    }
    super::unix::name_problem(name)
}

pub(crate) fn trash_one(path: &Path) -> Result<(), String> {
    trash::delete(path).map_err(|error| super::trash_reason::describe(&error))
}

pub(crate) fn trash_locations(_originals: &[PathBuf]) -> Vec<(PathBuf, PathBuf)> {
    Vec::new()
}

pub(crate) fn restore_trashed(paths: &[PathBuf]) -> Vec<Failure> {
    paths
        .iter()
        .map(|path| Failure {
            path: path.clone(),
            reason: NO_RESTORE.into(),
        })
        .collect()
}
