pub(super) fn os_bucket() -> &'static str {
    "macos"
}

#[cfg(test)]
pub(super) fn test_runtime_root() -> std::io::Result<tempfile::TempDir> {
    tempfile::tempdir_in("/tmp")
}
