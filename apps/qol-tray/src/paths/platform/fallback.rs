pub(super) fn os_bucket() -> &'static str {
    std::env::consts::OS
}

#[cfg(test)]
pub(super) fn test_runtime_root() -> std::io::Result<tempfile::TempDir> {
    tempfile::tempdir()
}
