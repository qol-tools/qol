use crate::daemon::Daemon;
#[cfg(feature = "dev")]
use crate::daemon::DiscoveredPluginInfo;
use crate::plugins::{ActionType, PluginManager};
use serde::{Deserialize, Serialize};
#[cfg(feature = "dev")]
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub(super) const DEFAULT_UI_SERVER_PORT: u16 = 42700;
pub(super) const MAX_COVER_SIZE: usize = 5 * 1024 * 1024;
pub(super) const MAX_CONFIG_SIZE: usize = 1024 * 1024;

#[cfg(feature = "dev")]
pub(super) const MOCK_TARGET_SELF_UPDATE: &str = "self_update";
#[cfg(feature = "dev")]
pub(super) const MOCK_TARGET_SELF_RECOMPILE: &str = "self_recompile";
#[cfg(feature = "dev")]
pub(super) const MOCK_TARGET_PLUGIN_BUILD: &str = "plugin_build";

#[derive(Clone)]
pub(super) struct AppState {
    pub(super) plugins_dir: PathBuf,
    pub(super) plugin_manager: Arc<Mutex<PluginManager>>,
    pub(super) daemon: Daemon,
}

#[derive(Serialize)]
pub(super) struct PluginInfo {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) description: String,
    pub(super) version: String,
    pub(super) installed: bool,
    pub(super) installed_version: Option<String>,
}

#[derive(Serialize)]
pub(super) struct PluginsResponse {
    pub(super) plugins: Vec<PluginInfo>,
    pub(super) cache_age_secs: Option<u64>,
}

#[derive(Deserialize, Default)]
pub(super) struct PluginsQuery {
    #[serde(default)]
    pub(super) refresh: bool,
}

#[derive(Serialize)]
pub(super) struct UninstallResult {
    pub(super) success: bool,
    pub(super) message: String,
}

#[derive(Serialize)]
pub(super) struct ExecuteActionResult {
    pub(super) success: bool,
    pub(super) message: String,
}

#[derive(Serialize)]
pub(super) struct PluginAction {
    pub(super) id: String,
    pub(super) label: String,
    pub(super) kind: ActionType,
}

#[derive(Serialize)]
pub(super) struct InstalledPlugin {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) description: String,
    pub(super) version: String,
    pub(super) loaded: bool,
    pub(super) load_error: Option<String>,
    pub(super) has_cover: bool,
    pub(super) has_ui: bool,
    pub(super) available_version: Option<String>,
    pub(super) update_available: bool,
    pub(super) actions: Vec<PluginAction>,
}

#[derive(Serialize)]
pub(super) struct InstalledPluginsResponse {
    pub(super) revision: u64,
    pub(super) plugins: Vec<InstalledPlugin>,
}

#[derive(Deserialize)]
pub(super) struct TokenRequest {
    pub(super) token: String,
}

#[derive(Serialize)]
pub(super) struct TokenStatus {
    pub(super) has_token: bool,
}

#[cfg(feature = "dev")]
#[derive(Debug, Clone, Serialize, Default)]
pub(super) struct BuildProgressSnapshot {
    pub(super) status: String,
    pub(super) percent: u8,
    pub(super) phase: String,
}

#[cfg(feature = "dev")]
#[derive(Debug, Clone, Serialize, Default)]
pub(super) struct BuildStateResponse {
    pub(super) building: bool,
    #[serde(default)]
    pub(super) progress: HashMap<String, BuildProgressSnapshot>,
}

#[cfg(feature = "dev")]
#[derive(Serialize)]
pub(super) struct DiscoveryStateResponse {
    pub(super) status: String,
    pub(super) plugins: Vec<DiscoveredPluginInfo>,
}

#[cfg(feature = "dev")]
#[derive(Serialize, Clone, Copy)]
pub(super) struct MockTargetInfo {
    pub(super) id: &'static str,
    pub(super) label: &'static str,
    pub(super) running: bool,
    pub(super) supports_stop: bool,
}

#[cfg(feature = "dev")]
#[derive(Debug, Clone, Deserialize)]
pub(super) struct UpsertPluginLogControlRequest {
    #[serde(default)]
    pub(super) muted: bool,
    #[serde(default)]
    pub(super) suppress_patterns: Vec<String>,
}
