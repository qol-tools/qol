pub(super) fn sync(
    _entries: &[super::super::LauncherEntry],
    _target: &std::path::Path,
    _marks: &super::super::icon::MarkFiles,
) -> anyhow::Result<()> {
    Ok(())
}

pub(super) fn publish_synced() {}
