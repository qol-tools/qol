#[cfg(feature = "dev")]
use super::dev_plugin_cpu::DevPluginCpuService;
#[cfg(feature = "dev")]
use super::dev_runtime::DevRuntimeService;
#[cfg(feature = "dev")]
use super::restart::RestartPort;
use crate::daemon::Daemon;
#[cfg(feature = "dev")]
use crate::dev::state::DiscoveredPluginInfo;
use crate::plugins::{ActionType, PluginId, PluginLoader, PluginManager};
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
    #[cfg(feature = "dev")]
    pub(super) dev_state: Arc<crate::dev::state::DevState>,
    #[cfg(feature = "dev")]
    pub(super) runtime: Arc<DevRuntimeService>,
    #[cfg(feature = "dev")]
    pub(super) plugin_cpu: Arc<DevPluginCpuService>,
    #[cfg(feature = "dev")]
    pub(super) restart: Arc<dyn RestartPort>,
    #[cfg(feature = "dev")]
    pub(super) core_log_controls:
        Arc<std::sync::RwLock<HashMap<String, crate::logging::LogControl>>>,
}

impl AppState {
    pub(super) fn new(
        plugin_manager: Arc<Mutex<PluginManager>>,
        daemon: &Daemon,
        #[cfg(feature = "dev")] core_log_controls: Arc<
            std::sync::RwLock<HashMap<String, crate::logging::LogControl>>,
        >,
    ) -> anyhow::Result<(Self, PathBuf)> {
        let plugins_dir = PluginLoader::default_plugin_dir()?;
        let state = Self {
            plugins_dir: plugins_dir.clone(),
            #[cfg(feature = "dev")]
            plugin_cpu: DevPluginCpuService::start(plugin_manager.clone(), daemon.events.clone()),
            plugin_manager,
            daemon: daemon.clone(),
            #[cfg(feature = "dev")]
            dev_state: Arc::new(crate::dev::state::DevState::new()),
            #[cfg(feature = "dev")]
            runtime: super::dev_runtime::new_dev_runtime(),
            #[cfg(feature = "dev")]
            restart: super::restart::default_restart_port(),
            #[cfg(feature = "dev")]
            core_log_controls,
        };
        Ok((state, plugins_dir))
    }
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
    pub(super) id: PluginId,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) results: Option<Vec<crate::dev::state::BuildResultInfo>>,
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

#[cfg(feature = "dev")]
#[derive(Debug, Clone, Deserialize, Default)]
pub(super) struct SetPluginCpuMonitoringRequest {
    #[serde(default)]
    pub(super) plugin_ids: Vec<String>,
}

#[cfg(feature = "dev")]
#[derive(Debug, Clone, Deserialize, Default)]
pub(super) struct RecompileSelfRequest {
    pub(super) worktree_path: Option<String>,
}

#[cfg(feature = "dev")]
#[derive(Debug, Clone, Deserialize, Default)]
pub(super) struct ReloadRequest {
    pub(super) worktree_path: Option<String>,
}

#[cfg(feature = "dev")]
#[derive(Debug, Clone, Serialize)]
pub(super) struct WorktreeInfo {
    pub(super) branch: String,
    pub(super) path: String,
}

#[cfg(all(test, feature = "dev"))]
mod tests {
    use super::RecompileSelfRequest;

    #[test]
    fn recompile_request_path_is_optional() {
        let cases = [
            (r#"{}"#, None),
            (r#"{"worktree_path":null}"#, None),
            (r#"{"worktree_path":"/a/b/c"}"#, Some("/a/b/c")),
        ];
        for (input, expected_path) in cases {
            let req: RecompileSelfRequest = serde_json::from_str(input)
                .unwrap_or_else(|e| panic!("failed to parse {:?}: {}", input, e));
            assert_eq!(
                req.worktree_path.as_deref(),
                expected_path,
                "input: {}",
                input
            );
        }
    }
}
