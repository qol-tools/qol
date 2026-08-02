pub mod git_repo;
mod merge;
mod promote;
mod reconcile;
mod scope;
mod service;
mod state;
mod types;

#[cfg(test)]
pub(crate) use promote::promote_allowlisted_clone;
pub(crate) use scope::SCOPE_REQUIREMENTS;
pub use service::SyncService;
pub use types::{
    ConflictChoice, ResolvableConflict, Side, SyncActionResult, SyncBackupEntry, SyncBackupPreview,
    SyncConnectRequest, SyncHealth, SyncIncident, SyncIncidentKind, SyncStatus,
};

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
