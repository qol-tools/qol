use super::super::LauncherEntry;

pub(super) fn sync(_entries: &[super::super::ResolvedEntry]) -> anyhow::Result<()> {
    anyhow::bail!("launcher application integration is unavailable on this platform")
}

pub(super) fn publish_synced() {}
