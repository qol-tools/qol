#[cfg(test)]
mod tests;

use std::sync::Arc;

use crate::ops::bin::Bin;
use crate::ops::NameProblem;

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

pub(crate) fn system_bin() -> Arc<dyn Bin> {
    super::bin::system_bin(NO_RESTORE)
}
