const DEFAULT_PATH: &str = "qol-tray/profile.json";
const DEFAULT_COMMIT_MESSAGE: &str = "chore: sync qol-tray profile";
const AUTO_PUSH_INTERVAL_SECS: u64 = 3;

mod platform;
mod providers;
mod service;
mod state;
mod types;

pub use service::SyncService;
pub use types::{
    GitHubSyncConnection, LocalFolderSyncConnection, SyncActionResult, SyncBackupEntry,
    SyncBackupPreview, SyncBranchList, SyncBranchListRequest, SyncConnectRequest, SyncConnection,
    SyncHealth, SyncIncident, SyncProviderDefinition, SyncProviderFieldDefinition,
    SyncProviderFieldKey, SyncProviderFieldKind, SyncProviderFieldOptionsSource,
    SyncProviderFieldSection, SyncProviderKind, SyncStatus,
};
