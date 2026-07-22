use super::super::LauncherEntry;

pub(super) fn sync(
    _entries: &[LauncherEntry],
    _binary_path: &std::path::Path,
) -> anyhow::Result<()> {
    anyhow::bail!("launcher application integration is unavailable on this platform")
}

pub(super) fn apps_dir() -> Option<std::path::PathBuf> {
    None
}
