pub(crate) mod scope;
mod service;

pub use qol_profile_sync::{
    ConflictChoice, ResolvableConflict, Side, SyncActionResult, SyncBackupEntry, SyncBackupPreview,
    SyncConnectRequest, SyncHealth, SyncIncident, SyncIncidentKind, SyncStatus,
};
pub(crate) use scope::SCOPE_REQUIREMENTS;
#[cfg(test)]
pub(crate) use service::promote_allowlisted_clone;
pub use service::SyncService;

pub(crate) fn open_path(path: &std::path::Path) -> anyhow::Result<()> {
    if !path.exists() {
        anyhow::bail!("Path does not exist");
    }
    qol_apps::desktop_integration::open_with_default_app(path)?;
    Ok(())
}

pub(crate) fn open_dir(path: &std::path::Path) -> anyhow::Result<()> {
    open_path(path)
}
