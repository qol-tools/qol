pub(super) fn sync(
    _entries: &[super::super::LauncherEntry],
    _binary_path: &std::path::Path,
) -> anyhow::Result<()> {
    Ok(())
}

pub(super) fn apps_dir() -> Option<std::path::PathBuf> {
    None
}
