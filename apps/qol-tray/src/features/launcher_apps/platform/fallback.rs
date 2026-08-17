use super::super::LauncherEntry;

use std::path::Path;

pub(super) fn sync(_entries: &[super::super::LauncherEntry], _target: &Path) -> anyhow::Result<()> {
    anyhow::bail!("launcher application integration is unavailable on this platform")
}

pub(super) fn publish_synced() {}
