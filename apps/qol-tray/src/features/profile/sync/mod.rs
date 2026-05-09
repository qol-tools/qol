const DEFAULT_PATH: &str = "qol-tray/profile.json";
const AUTO_PUSH_INTERVAL_SECS: u64 = 3;

pub(crate) mod platform;
mod providers;
mod resolve;
mod service;
mod state;
mod types;

pub use service::SyncService;
pub use types::{
    GitHubSyncConnection, LocalFolderSyncConnection, SyncActionResult, SyncBackupEntry,
    SyncBackupPreview, SyncConnectRequest, SyncConnection, SyncHealth, SyncIncident,
    SyncIncidentKind, SyncProviderDefinition, SyncProviderFieldDefinition, SyncProviderFieldKey,
    SyncProviderFieldKind, SyncProviderFieldSection, SyncProviderKind, SyncStatus,
};
