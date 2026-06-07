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
use axum::extract::FromRef;
use serde::{Deserialize, Serialize};
#[cfg(feature = "dev")]
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, RwLock};

use super::super::github::PluginCache;

pub(super) type InstalledCache = Arc<Mutex<Option<(u64, Arc<InstalledPluginsResponse>)>>>;

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
    pub(super) github_auth_service: Arc<crate::features::github_auth::GitHubAuthService>,
    pub(super) sync_service: Arc<crate::features::profile::sync::SyncService>,
    pub(super) installed_cache: InstalledCache,
    pub(super) plugins_cache: Arc<RwLock<Option<PluginCache>>>,
    pub(super) plugins_revalidating: Arc<AtomicBool>,
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
        github_auth_service: Arc<crate::features::github_auth::GitHubAuthService>,
        sync_service: Arc<crate::features::profile::sync::SyncService>,
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
            github_auth_service,
            sync_service,
            installed_cache: Arc::new(Mutex::new(None)),
            plugins_cache: Arc::new(RwLock::new(super::super::github::read_cache())),
            plugins_revalidating: Arc::new(AtomicBool::new(false)),
            #[cfg(feature = "dev")]
            dev_state: Arc::new(crate::dev::state::DevState::new()),
            #[cfg(feature = "dev")]
            runtime: super::dev_runtime::new_dev_runtime(),
            #[cfg(feature = "dev")]
            restart: super::restart::default_restart_port(),
            #[cfg(feature = "dev")]
            core_log_controls,
        };
        if let Ok(manager) = state.plugin_manager.lock() {
            if let Err(error) =
                crate::features::profile::core::sync_plugins_lock_from_plugins(manager.plugins())
            {
                log::error!("Failed to sync profile plugins lock on startup: {}", error);
            }
        }
        Ok((state, plugins_dir))
    }
}

impl FromRef<AppState> for crate::features::profile::http::ProfileHttpState {
    fn from_ref(state: &AppState) -> Self {
        Self {
            plugins_dir: state.plugins_dir.clone(),
            plugin_manager: state.plugin_manager.clone(),
            daemon: state.daemon.clone(),
            sync_service: state.sync_service.clone(),
        }
    }
}

impl FromRef<AppState> for crate::features::github_auth::GitHubAuthHttpState {
    fn from_ref(state: &AppState) -> Self {
        Self {
            github_auth_service: state.github_auth_service.clone(),
        }
    }
}

impl FromRef<AppState> for crate::features::auth::AuthHttpState {
    fn from_ref(state: &AppState) -> Self {
        Self {
            github_auth_service: state.github_auth_service.clone(),
        }
    }
}

#[derive(Clone, Serialize)]
pub(super) struct PluginInfo {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) description: String,
    pub(super) version: String,
    pub(super) installed: bool,
    pub(super) installed_version: Option<String>,
}

#[derive(Clone, Serialize)]
pub(super) struct PluginsResponse {
    pub(super) plugins: Vec<PluginInfo>,
    pub(super) cache_age_secs: Option<u64>,
    #[serde(default)]
    pub(super) stale: bool,
    #[serde(default)]
    pub(super) revalidating: bool,
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
pub(super) struct PluginPermissionsResponse {
    pub(super) permissions:
        std::collections::HashMap<String, crate::plugins::capabilities::PermissionStatus>,
}

#[derive(Clone, Serialize)]
pub(super) struct PluginAction {
    pub(super) id: String,
    pub(super) label: String,
    pub(super) kind: ActionType,
}

#[derive(Clone, Serialize)]
pub(super) struct InstalledPlugin {
    pub(super) id: PluginId,
    pub(super) name: String,
    pub(super) description: String,
    pub(super) version: String,
    pub(super) loaded: bool,
    pub(super) load_error: Option<String>,
    pub(super) has_cover: bool,
    pub(super) has_custom_ui: bool,
    pub(super) has_config: bool,
    pub(super) available_version: Option<String>,
    pub(super) update_available: bool,
    pub(super) actions: Vec<PluginAction>,
    pub(super) source: Option<&'static str>,
    pub(super) resolved_from: Option<&'static str>,
    pub(super) active_failure_reason: Option<String>,
    pub(super) unavailable: bool,
}

#[derive(Clone, Serialize)]
pub(super) struct InstalledPluginsResponse {
    pub(super) revision: u64,
    pub(super) plugins: Vec<InstalledPlugin>,
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
    pub(super) worktree_branch: Option<String>,
}

#[cfg(feature = "dev")]
#[derive(Debug, Clone, Deserialize, Default)]
pub(super) struct ReloadRequest {
    pub(super) worktree_branch: Option<String>,
}

#[cfg(feature = "dev")]
#[derive(Debug, Clone, Serialize)]
pub(super) struct WorktreeInfo {
    pub(super) branch: String,
    pub(super) path: String,
}

#[cfg(feature = "dev")]
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(super) struct ActiveWorktreeResponse {
    pub(super) branch: Option<String>,
    #[serde(rename = "repoBranch")]
    pub(super) repo_branch: Option<String>,
}

#[cfg(feature = "dev")]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub(super) struct ToolingGhAccountPayload {
    #[serde(default)]
    pub(super) value: Option<String>,
}

#[cfg(feature = "dev")]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub(super) struct RuntimeGpuiPayload {
    #[serde(default)]
    pub(super) ghost_opacity: Option<f32>,
    #[serde(default)]
    pub(super) ghost_debug_color: Option<String>,
}

#[cfg(all(test, feature = "dev"))]
mod tests {
    use super::RecompileSelfRequest;

    #[test]
    fn recompile_request_branch_is_optional() {
        let cases = [
            (r#"{}"#, None),
            (r#"{"worktree_branch":null}"#, None),
            (r#"{"worktree_branch":"feat/x"}"#, Some("feat/x")),
        ];
        for (input, expected_branch) in cases {
            let req: RecompileSelfRequest = serde_json::from_str(input)
                .unwrap_or_else(|e| panic!("failed to parse {:?}: {}", input, e));
            assert_eq!(
                req.worktree_branch.as_deref(),
                expected_branch,
                "input: {}",
                input
            );
        }
    }
}
