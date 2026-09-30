use super::super::LauncherEntry;

use std::path::Path;

pub(super) fn sync(
    _entries: &[super::super::LauncherEntry],
    _target: &Path,
    _marks: &super::super::icon::MarkFiles,
) -> anyhow::Result<()> {
    anyhow::bail!("launcher application integration is unavailable on this platform")
}

pub(super) fn publish_synced() {}
