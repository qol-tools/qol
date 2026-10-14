use std::path::{Path, PathBuf};

pub(super) fn install_candidates(_home: Option<&Path>) -> Vec<PathBuf> {
    Vec::new()
}

pub(super) fn is_executable(path: &Path) -> bool {
    path.is_file()
}
