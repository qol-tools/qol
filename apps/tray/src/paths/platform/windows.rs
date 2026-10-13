pub(super) fn os_bucket() -> &'static str {
    "windows"
}

#[cfg(not(test))]
pub(in crate::paths) fn fallback_runtime_dir() -> Option<std::path::PathBuf> {
    None
}

#[cfg(test)]
pub(super) fn test_runtime_root() -> std::io::Result<tempfile::TempDir> {
    tempfile::tempdir()
}
