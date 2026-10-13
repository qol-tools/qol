pub(super) fn os_bucket() -> &'static str {
    "macos"
}

#[cfg(not(test))]
pub(in crate::paths) fn fallback_runtime_dir() -> Option<std::path::PathBuf> {
    Some(std::path::PathBuf::from(qol_conventions::RUNTIME_DIR_PATH))
}

#[cfg(test)]
pub(super) fn test_runtime_root() -> std::io::Result<tempfile::TempDir> {
    tempfile::tempdir_in("/tmp")
}
