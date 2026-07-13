use std::path::{Path, PathBuf};

pub(crate) fn bundle_info(_: &Path) -> (Option<String>, Option<String>) {
    (None, None)
}

pub(crate) fn spotlight_app_paths(_: &[PathBuf]) -> Vec<PathBuf> {
    Vec::new()
}
