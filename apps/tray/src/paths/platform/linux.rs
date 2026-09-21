pub(super) fn os_bucket() -> &'static str {
    "linux"
}

#[cfg(test)]
pub(super) fn test_runtime_root() -> std::io::Result<tempfile::TempDir> {
    tempfile::tempdir()
}
