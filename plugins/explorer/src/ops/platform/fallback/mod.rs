use std::io;
use std::path::Path;
use std::sync::Arc;

use crate::ops::bin::{Bin, BinError, Held};
use crate::ops::NameProblem;

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

struct NoBin;

impl Bin for NoBin {
    fn trash(&self, _path: &Path) -> Result<Option<Held>, BinError> {
        Err(BinError::NoTrash(NO_TRASH.into()))
    }

    fn restore(&self, _held: &Held, _to: &Path) -> Result<(), BinError> {
        Err(BinError::NoTrash(NO_TRASH.into()))
    }
}

pub(crate) fn system_bin() -> Arc<dyn Bin> {
    Arc::new(NoBin)
}
