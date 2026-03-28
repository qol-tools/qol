use axum::{extract::State, http::StatusCode, response::IntoResponse, response::Response, Json};
use serde::Serialize;
use serde_json::Value;

use super::super::helpers::reload_manager_and_notify_without_profile_sync;
use super::super::types::AppState;
use crate::features::plugin_store::installer::PluginInstaller;
use crate::plugins::PluginConfigManager;

#[derive(Serialize)]
struct ImportPluginResult {
    id: String,
    status: String,
    message: String,
}

#[derive(Serialize)]
struct ImportResult {
    success: bool,
    plugins: Vec<ImportPluginResult>,
}

pub(in super::super) async fn export_config(State(state): State<AppState>) -> impl IntoResponse {
    let plugins = export_plugins(&state);
    let bundle = crate::profile::ProfileExportBundle {
        version: crate::profile::CURRENT_PROFILE_VERSION,
        exported_at: chrono::Local::now().to_rfc3339(),
        hotkeys: read_json_file_or_default(crate::paths::hotkeys_path()),
        shortcuts: read_json_file_or_default(crate::paths::shortcuts_path()),
        task_runner: read_json_file_or_default(crate::paths::task_runner_config_path()),
        plugin_configs: crate::profile::read_plugin_configs().unwrap_or_default(),
        plugins,
    };
    Json(bundle).into_response()
}

pub(in super::super) async fn import_config(
    State(state): State<AppState>,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    let bundle: crate::profile::ProfileImportBundle = match serde_json::from_slice(&body) {
        Ok(bundle) => bundle,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    match import_bundle(&state, bundle).await {
        Ok(result) => Json(result).into_response(),
        Err(response) => response,
    }
}

async fn import_bundle(
    state: &AppState,
    bundle: crate::profile::ProfileImportBundle,
) -> Result<ImportResult, Response> {
    crate::profile::ensure_profile_dirs().map_err(server_error)?;
    write_core_settings(&bundle).map_err(server_error)?;
    if let Some(plugin_configs) = &bundle.plugin_configs {
        crate::profile::replace_plugin_configs(plugin_configs).map_err(server_error)?;
    }

    let plugins = crate::profile::import_plugins(&bundle);
    crate::profile::save_plugins_lock(&crate::profile::PluginsLock {
        version: crate::profile::CURRENT_PROFILE_VERSION,
        plugins: plugins.clone(),
    })
    .map_err(server_error)?;

    let plugin_results = reconcile_plugins(state, &plugins).await;
    project_plugin_configs(state, bundle.plugin_configs.as_ref()).map_err(server_error)?;
    reload_manager_and_notify_without_profile_sync(state);

    let success = plugin_results
        .iter()
        .all(|result| result.status != "failed");
    Ok(ImportResult {
        success,
        plugins: plugin_results,
    })
}

fn export_plugins(state: &AppState) -> Vec<crate::profile::PluginLockEntry> {
    let Ok(manager) = state.plugin_manager.lock() else {
        return crate::profile::load_plugins_lock()
            .map(|lock| lock.plugins)
            .unwrap_or_default();
    };
    crate::profile::sync_plugins_lock_from_plugins(manager.plugins())
        .map(|lock| lock.plugins)
        .unwrap_or_else(|_| {
            crate::profile::load_plugins_lock()
                .map(|lock| lock.plugins)
                .unwrap_or_default()
        })
}

fn write_core_settings(bundle: &crate::profile::ProfileImportBundle) -> anyhow::Result<()> {
    if let Some(hotkeys) = &bundle.hotkeys {
        write_json_config(crate::paths::hotkeys_path()?, hotkeys)?;
    }
    if let Some(shortcuts) = &bundle.shortcuts {
        write_json_config(crate::paths::shortcuts_path()?, shortcuts)?;
    }
    if let Some(task_runner) = &bundle.task_runner {
        write_json_config(crate::paths::task_runner_config_path()?, task_runner)?;
    }
    Ok(())
}

async fn reconcile_plugins(
    state: &AppState,
    plugins: &[crate::profile::PluginLockEntry],
) -> Vec<ImportPluginResult> {
    let installer = PluginInstaller::new(state.plugins_dir.clone());
    let mut results = Vec::new();

    for plugin in plugins {
        if !crate::plugins::manifest::supports_current_platform(&plugin.platforms) {
            results.push(ImportPluginResult {
                id: plugin.id.clone(),
                status: "skipped".to_string(),
                message: "unsupported on this platform".to_string(),
            });
            continue;
        }

        let plugin_dir = state.plugins_dir.join(&plugin.id);
        let current_version = super::super::helpers::read_plugin_version(&plugin_dir).ok();
        if current_version.as_deref() == Some(plugin.version.as_str()) && !plugin.version.is_empty()
        {
            results.push(ImportPluginResult {
                id: plugin.id.clone(),
                status: "kept".to_string(),
                message: format!("already at {}", plugin.version),
            });
            continue;
        }

        let result = restore_plugin(&installer, &plugin_dir, plugin).await;
        results.push(result);
    }

    results
}

async fn restore_plugin(
    installer: &PluginInstaller,
    plugin_dir: &std::path::Path,
    plugin: &crate::profile::PluginLockEntry,
) -> ImportPluginResult {
    let exists = plugin_dir.exists();
    let action = if exists { "update" } else { "install" };
    let result = if plugin.version.is_empty() {
        restore_latest(installer, &plugin.repo_url, &plugin.id, exists).await
    } else {
        restore_exact(
            installer,
            &plugin.repo_url,
            &plugin.id,
            &plugin.version,
            exists,
        )
        .await
    };

    let error = result.err();
    if let Some(error) = error {
        return ImportPluginResult {
            id: plugin.id.clone(),
            status: "failed".to_string(),
            message: format!("{action} failed: {error:#}"),
        };
    }

    let message = plugin_restore_message(plugin, action);
    ImportPluginResult {
        id: plugin.id.clone(),
        status: action.to_string(),
        message,
    }
}

async fn restore_latest(
    installer: &PluginInstaller,
    repo_url: &str,
    plugin_id: &str,
    exists: bool,
) -> anyhow::Result<()> {
    if exists {
        return installer.update(repo_url, plugin_id).await;
    }
    installer.install(repo_url, plugin_id).await
}

async fn restore_exact(
    installer: &PluginInstaller,
    repo_url: &str,
    plugin_id: &str,
    version: &str,
    exists: bool,
) -> anyhow::Result<()> {
    if exists {
        return installer.update_exact(repo_url, plugin_id, version).await;
    }
    installer.install_exact(repo_url, plugin_id, version).await
}

fn plugin_restore_message(plugin: &crate::profile::PluginLockEntry, action: &str) -> String {
    let verb = match action {
        "update" => "updated",
        _ => "installed",
    };
    if plugin.version.is_empty() {
        return format!("{verb} latest available version");
    }
    format!("{verb} {}", plugin.version)
}

fn project_plugin_configs(
    state: &AppState,
    plugin_configs: Option<&std::collections::HashMap<String, Value>>,
) -> anyhow::Result<()> {
    let Some(plugin_configs) = plugin_configs else {
        return Ok(());
    };
    let manager = PluginConfigManager::new()?;
    for (plugin_id, config) in plugin_configs {
        if !state.plugins_dir.join(plugin_id).is_dir() {
            continue;
        }
        manager.set_config(plugin_id, config.clone())?;
    }
    Ok(())
}

fn read_json_file_or_default(path: anyhow::Result<std::path::PathBuf>) -> Value {
    let Ok(path) = path else {
        return Value::Null;
    };
    match std::fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or(Value::Null),
        Err(_) => Value::Null,
    }
}

fn write_json_config(path: std::path::PathBuf, value: &Value) -> anyhow::Result<()> {
    let content = serde_json::to_vec_pretty(value)?;
    crate::file_io::ensure_parent_dir(&path)?;
    crate::file_io::atomic_write(&path, &content)
}

fn server_error(error: anyhow::Error) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("Failed to import profile: {error:#}"),
    )
        .into_response()
}
