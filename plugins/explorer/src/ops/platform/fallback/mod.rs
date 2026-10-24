use std::io;
use std::path::{Path, PathBuf};

use crate::ops::{Failure, NameProblem};

const MAX_NAME_BYTES: usize = 255;
const NO_TRASH: &str = "this system has no Trash to use";

pub(crate) fn free_bytes(_path: &Path) -> io::Result<u64> {
    Err(io::ErrorKind::Unsupported.into())
}

pub(crate) fn volume_of(_path: &Path) -> io::Result<u64> {
    Err(io::ErrorKind::Unsupported.into())
}

pub(crate) fn removable(_path: &Path) -> Result<(), String> {
    Ok(())
}

pub(crate) fn name_problem(name: &str) -> Option<NameProblem> {
    if name.trim().is_empty() {
        return Some(NameProblem::Empty);
    }
    if name.contains('/') || name.contains('\0') {
        return Some(NameProblem::Forbidden(
            "/ and null characters cannot be in a name".into(),
        ));
    }
    if name == "." || name == ".." {
        return Some(NameProblem::Forbidden("a name cannot be . or ..".into()));
    }
    if name.len() > MAX_NAME_BYTES {
        return Some(NameProblem::TooLong);
    }
    None
}

pub(crate) fn copy_link(_from: &Path, _to: &Path) -> io::Result<()> {
    Err(io::ErrorKind::Unsupported.into())
}

pub(crate) fn trash_one(_path: &Path) -> Result<(), String> {
    Err(NO_TRASH.into())
}

pub(crate) fn trash_locations(_originals: &[PathBuf]) -> Vec<(PathBuf, PathBuf)> {
    Vec::new()
}

pub(crate) fn restore_trashed(paths: &[PathBuf]) -> Vec<Failure> {
    paths
        .iter()
        .map(|path| Failure {
            path: path.clone(),
            reason: NO_TRASH.into(),
        })
        .collect()
}

pub(crate) fn in_trash(_path: &Path) -> bool {
    false
}
