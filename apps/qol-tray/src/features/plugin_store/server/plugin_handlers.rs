use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};

use crate::paths::is_safe_path_component;
use crate::plugins::PluginLoader;

use super::helpers::{
    extract_actions, infer_load_error, is_newer_version, read_installed_plugin_dirs,
    read_manifest_without_validation, read_plugin_version, reload_manager_and_notify,
};
use super::types::{
    AppState, ExecuteActionResult, InstalledPlugin, InstalledPluginsResponse, PluginInfo,
    PluginsQuery, PluginsResponse, UninstallResult,
};

pub(super) async fn list_plugins(
    Query(query): Query<PluginsQuery>,
) -> Result<Json<PluginsResponse>, (StatusCode, String)> {
    use super::super::github::{cache_age_secs, GitHubClient};

    log::info!("API /plugins called (refresh={})", query.refresh);

    let client = GitHubClient::new("qol-tools");
    let plugins_dir = match PluginLoader::default_plugin_dir() {
        Ok(dir) => dir,
        Err(e) => {
            log::error!("Failed to determine config directory: {}", e);
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to determine plugin directory".to_string(),
            ));
        }
    };

    let installed_versions: std::collections::HashMap<String, String> =
        read_installed_plugin_dirs(&plugins_dir)
            .into_iter()
            .filter_map(|(id, path)| {
                let version = read_plugin_version(&path).ok()?;
                Some((id, version))
            })
            .collect();

    let cache_age = cache_age_secs();

    let metadata_list = client
        .list_plugins_cached(query.refresh)
        .await
        .map_err(|e| {
            log::error!("Failed to fetch plugins: {}", e);
            (
                StatusCode::SERVICE_UNAVAILABLE,
                format!("Failed to fetch plugins: {:#}", e),
            )
        })?;
    log::info!("Got {} plugins", metadata_list.len());
    let plugins = metadata_list
        .into_iter()
        .filter(|m| m.supports_current_platform())
        .map(|m| {
            let installed_version = installed_versions.get(&m.id).cloned();
            PluginInfo {
                id: m.id.clone(),
                name: m.name,
                description: m.description,
                version: m.version,
                installed: installed_version.is_some(),
                installed_version,
            }
        })
        .collect();

    Ok(Json(PluginsResponse {
        plugins,
        cache_age_secs: cache_age,
    }))
}

pub(super) async fn install_plugin(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<PluginInfo>, (StatusCode, String)> {
    use super::super::installer::PluginInstaller;

    if !is_safe_path_component(&id) {
        return Err((StatusCode::BAD_REQUEST, "Invalid plugin ID".to_string()));
    }

    log::info!("Install requested for plugin: {}", id);

    if let Err(e) = std::fs::create_dir_all(&state.plugins_dir) {
        log::error!("Failed to get plugins directory: {}", e);
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to access plugins directory".to_string(),
        ));
    }

    let plugins_dir = state.plugins_dir.clone();

    let installer = PluginInstaller::new(plugins_dir.clone());
    let repo_url = format!("https://github.com/qol-tools/{}.git", id);

    installer.install(&repo_url, &id).await.map_err(|e| {
        log::error!("Failed to install plugin {}: {}", id, e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Installation failed: {:#}", e),
        )
    })?;

    reload_manager_and_notify(&state);

    log::info!("Plugin {} installed successfully", id);
    let version = read_plugin_version(&plugins_dir.join(&id)).unwrap_or_else(|_| "unknown".into());
    Ok(Json(PluginInfo {
        id: id.clone(),
        name: id.clone(),
        description: "Installed successfully".to_string(),
        installed_version: Some(version.clone()),
        version,
        installed: true,
    }))
}

pub(super) async fn update_plugin(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> Json<UninstallResult> {
    use super::super::installer::PluginInstaller;

    if !is_safe_path_component(&id) {
        return Json(UninstallResult {
            success: false,
            message: "Invalid plugin ID".to_string(),
        });
    }

    log::info!("Update requested for plugin: {}", id);

    let installer = PluginInstaller::new(state.plugins_dir.clone());
    let repo_url = format!("https://github.com/qol-tools/{}.git", id);

    if let Err(e) = installer.update(&repo_url, &id).await {
        log::error!("Failed to update plugin {}: {}", id, e);
        return Json(UninstallResult {
            success: false,
            message: format!("Update failed: {:#}", e),
        });
    }

    if let Ok(version) = read_plugin_version(&state.plugins_dir.join(&id)) {
        super::super::github::update_cached_version(&id, &version);
    }

    reload_manager_and_notify(&state);

    log::info!("Plugin {} updated successfully", id);
    Json(UninstallResult {
        success: true,
        message: "Updated successfully".to_string(),
    })
}

pub(super) async fn uninstall_plugin(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> Json<UninstallResult> {
    use super::super::installer::PluginInstaller;

    if !is_safe_path_component(&id) {
        return Json(UninstallResult {
            success: false,
            message: "Invalid plugin ID".to_string(),
        });
    }

    log::info!("Uninstall requested for plugin: {}", id);

    let installer = PluginInstaller::new(state.plugins_dir.clone());

    if let Err(e) = installer.uninstall(&id).await {
        log::error!("Failed to uninstall plugin {}: {}", id, e);
        return Json(UninstallResult {
            success: false,
            message: "Uninstall failed".to_string(),
        });
    }

    reload_manager_and_notify(&state);

    log::info!("Plugin {} uninstalled successfully", id);
    Json(UninstallResult {
        success: true,
        message: "Uninstalled successfully".to_string(),
    })
}

pub(super) async fn execute_plugin_action(
    Path((id, action)): Path<(String, String)>,
    State(state): State<AppState>,
) -> (StatusCode, Json<ExecuteActionResult>) {
    if !is_safe_path_component(&id) {
        return (
            StatusCode::BAD_REQUEST,
            Json(ExecuteActionResult {
                success: false,
                message: "Invalid plugin ID".to_string(),
            }),
        );
    }

    match crate::plugins::action_executor::try_execute_action(&state.plugin_manager, &id, &action) {
        Ok(()) => (
            StatusCode::OK,
            Json(ExecuteActionResult {
                success: true,
                message: "Action dispatched".to_string(),
            }),
        ),
        Err(error @ crate::plugins::action_executor::ActionExecutionError::PluginNotFound(_)) => {
            log::warn!("Plugin action rejected for {}::{}: {}", id, action, error);
            (
                StatusCode::NOT_FOUND,
                Json(ExecuteActionResult {
                    success: false,
                    message: error.to_string(),
                }),
            )
        }
        Err(error @ crate::plugins::action_executor::ActionExecutionError::InvalidActionId(_)) => {
            log::warn!("Plugin action rejected for {}::{}: {}", id, action, error);
            (
                StatusCode::BAD_REQUEST,
                Json(ExecuteActionResult {
                    success: false,
                    message: error.to_string(),
                }),
            )
        }
        Err(
            error @ crate::plugins::action_executor::ActionExecutionError::MissingActionMapping {
                ..
            },
        ) => {
            log::warn!("Plugin action rejected for {}::{}: {}", id, action, error);
            (
                StatusCode::BAD_REQUEST,
                Json(ExecuteActionResult {
                    success: false,
                    message: error.to_string(),
                }),
            )
        }
        Err(error) => {
            log::error!("Plugin action failed for {}::{}: {}", id, action, error);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ExecuteActionResult {
                    success: false,
                    message: "Action execution failed".to_string(),
                }),
            )
        }
    }
}

pub(super) async fn list_installed(
    State(state): State<AppState>,
) -> Result<Json<InstalledPluginsResponse>, StatusCode> {
    use super::super::github::read_cache;
    use std::collections::HashMap;

    let revision = state.daemon.events.plugins_revision();

    let manager = state.plugin_manager.lock().map_err(|e| {
        log::error!("Plugin manager mutex poisoned: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let cached_versions: HashMap<String, String> = read_cache()
        .map(|c| c.plugins.into_iter().map(|p| (p.id, p.version)).collect())
        .unwrap_or_default();

    let mut plugins_by_id: HashMap<String, InstalledPlugin> = manager
        .plugins()
        .map(|plugin| {
            let cover_path = plugin.path.join("cover.png");
            let ui_path = plugin.path.join("ui").join("index.html");
            let available_version = cached_versions.get(&plugin.id).cloned();
            let update_available = available_version
                .as_ref()
                .map(|av| is_newer_version(av, &plugin.manifest.plugin.version))
                .unwrap_or(false);

            let actions = extract_actions(&plugin.manifest.menu.items);

            (
                plugin.id.clone(),
                InstalledPlugin {
                    id: plugin.id.clone(),
                    name: plugin.manifest.plugin.name.clone(),
                    description: plugin.manifest.plugin.description.clone(),
                    version: plugin.manifest.plugin.version.clone(),
                    loaded: true,
                    load_error: None,
                    has_cover: cover_path.exists(),
                    has_ui: ui_path.exists(),
                    available_version,
                    update_available,
                    actions,
                },
            )
        })
        .collect();

    drop(manager);

    for (id, plugin_dir) in read_installed_plugin_dirs(&state.plugins_dir) {
        if plugins_by_id.contains_key(&id) {
            continue;
        }

        let manifest = read_manifest_without_validation(&plugin_dir);
        let (name, description, version, actions) = match manifest.as_ref() {
            Some(m) => (
                m.plugin.name.clone(),
                m.plugin.description.clone(),
                m.plugin.version.clone(),
                extract_actions(&m.menu.items),
            ),
            None => (
                id.clone(),
                "Plugin manifest could not be parsed".to_string(),
                "unknown".to_string(),
                Vec::new(),
            ),
        };

        let available_version = cached_versions.get(&id).cloned();
        let update_available = available_version
            .as_ref()
            .map(|av| version != "unknown" && is_newer_version(av, &version))
            .unwrap_or(false);

        let load_error = infer_load_error(&id, &plugin_dir, manifest.as_ref());

        plugins_by_id.insert(
            id.clone(),
            InstalledPlugin {
                id,
                name,
                description,
                version,
                loaded: false,
                load_error,
                has_cover: plugin_dir.join("cover.png").exists(),
                has_ui: plugin_dir.join("ui").join("index.html").exists(),
                available_version,
                update_available,
                actions,
            },
        );
    }

    let plugins: Vec<InstalledPlugin> = plugins_by_id.into_values().collect();

    Ok(Json(InstalledPluginsResponse { revision, plugins }))
}

pub(super) async fn sse_handler(State(state): State<AppState>) -> impl IntoResponse {
    use axum::response::sse::{Event, KeepAlive, Sse};
    use tokio_stream::wrappers::BroadcastStream;
    use tokio_stream::StreamExt;

    let rx = state.daemon.events.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|result| {
        result.ok().and_then(|event| {
            serde_json::to_string(&event)
                .ok()
                .map(|json| Ok::<_, std::convert::Infallible>(Event::default().data(json)))
        })
    });

    Sse::new(stream).keep_alive(KeepAlive::default())
}
