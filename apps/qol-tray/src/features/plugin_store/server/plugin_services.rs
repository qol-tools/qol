use axum::http::StatusCode;

use crate::plugins::PluginLoader;

use super::helpers::{
    extract_actions, infer_load_error, is_newer_version, read_installed_plugin_dirs,
    read_manifest_without_validation, read_plugin_version, reload_manager_and_notify,
};
use super::types::{
    AppState, InstalledPlugin, InstalledPluginsResponse, PluginInfo, PluginsResponse,
    UninstallResult,
};

#[cfg(feature = "dev")]
fn unlink_dev_plugin_if_linked(plugin_id: &str) -> Result<bool, String> {
    let config_dir = crate::paths::shared_config_dir().map_err(|e| e.to_string())?;
    match crate::dev::remove_link(plugin_id, &config_dir) {
        Ok(()) => Ok(true),
        Err(error) if error.contains("not dev-linked") => Ok(false),
        Err(error) => Err(error),
    }
}

#[cfg(not(feature = "dev"))]
fn unlink_dev_plugin_if_linked(_plugin_id: &str) -> Result<bool, String> {
    Ok(false)
}

pub(super) async fn list_plugins(refresh: bool) -> Result<PluginsResponse, (StatusCode, String)> {
    use super::super::github::{cache_age_secs, GitHubClient};

    log::info!("API /plugins called (refresh={})", refresh);

    let client = GitHubClient::new("qol-tools");
    let plugins_dir = match PluginLoader::default_plugin_dir() {
        Ok(dir) => dir,
        Err(error) => {
            log::error!("Failed to determine config directory: {}", error);
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

    let metadata_list = client.list_plugins_cached(refresh).await.map_err(|error| {
        log::error!("Failed to fetch plugins: {}", error);
        (
            StatusCode::SERVICE_UNAVAILABLE,
            format!("Failed to fetch plugins: {:#}", error),
        )
    })?;

    log::info!("Got {} plugins", metadata_list.len());

    let plugins = metadata_list
        .into_iter()
        .filter(|metadata| metadata.supports_current_platform())
        .map(|metadata| {
            let installed_version = installed_versions.get(&metadata.id).cloned();
            PluginInfo {
                id: metadata.id.clone(),
                name: metadata.name,
                description: metadata.description,
                version: metadata.version,
                installed: installed_version.is_some(),
                installed_version,
            }
        })
        .collect();

    Ok(PluginsResponse {
        plugins,
        cache_age_secs: cache_age,
    })
}

pub(super) async fn install_plugin(
    state: &AppState,
    id: &str,
) -> Result<PluginInfo, (StatusCode, String)> {
    use super::super::installer::PluginInstaller;

    log::info!("Install requested for plugin: {}", id);

    if let Err(error) = std::fs::create_dir_all(&state.plugins_dir) {
        log::error!("Failed to get plugins directory: {}", error);
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to access plugins directory".to_string(),
        ));
    }

    let plugins_dir = state.plugins_dir.clone();
    let installer = PluginInstaller::new(plugins_dir.clone());
    let repo_url = format!("https://github.com/qol-tools/{}.git", id);

    installer.install(&repo_url, id).await.map_err(|error| {
        log::error!("Failed to install plugin {}: {}", id, error);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Installation failed: {:#}", error),
        )
    })?;

    reload_manager_and_notify(state);

    log::info!("Plugin {} installed successfully", id);
    let version = read_plugin_version(&plugins_dir.join(id)).unwrap_or_else(|_| "unknown".into());

    Ok(PluginInfo {
        id: id.to_string(),
        name: id.to_string(),
        description: "Installed successfully".to_string(),
        installed_version: Some(version.clone()),
        version,
        installed: true,
    })
}

pub(super) async fn update_plugin(state: &AppState, id: &str) -> UninstallResult {
    use super::super::installer::PluginInstaller;

    log::info!("Update requested for plugin: {}", id);

    let installer = PluginInstaller::new(state.plugins_dir.clone());
    let repo_url = format!("https://github.com/qol-tools/{}.git", id);

    if let Err(error) = installer.update(&repo_url, id).await {
        log::error!("Failed to update plugin {}: {}", id, error);
        return UninstallResult {
            success: false,
            message: format!("Update failed: {:#}", error),
        };
    }

    if let Ok(version) = read_plugin_version(&state.plugins_dir.join(id)) {
        super::super::github::update_cached_version(id, &version);
    }

    reload_manager_and_notify(state);

    log::info!("Plugin {} updated successfully", id);
    UninstallResult {
        success: true,
        message: "Updated successfully".to_string(),
    }
}

pub(super) async fn uninstall_plugin(state: &AppState, id: &str) -> UninstallResult {
    use super::super::installer::PluginInstaller;

    log::info!("Uninstall requested for plugin: {}", id);

    let unlinked_dev = match unlink_dev_plugin_if_linked(id) {
        Ok(value) => value,
        Err(error) => {
            log::error!("Failed to unlink dev-linked plugin {}: {}", id, error);
            return UninstallResult {
                success: false,
                message: format!("Failed to unlink dev-linked plugin: {}", error),
            };
        }
    };

    let installer = PluginInstaller::new(state.plugins_dir.clone());

    let mut removed_installed_copy = false;
    match installer.uninstall(id).await {
        Ok(()) => {
            removed_installed_copy = true;
        }
        Err(error) => {
            let not_installed = error.to_string().contains("Plugin not installed");
            if !(unlinked_dev && not_installed) {
                log::error!("Failed to uninstall plugin {}: {}", id, error);
                return UninstallResult {
                    success: false,
                    message: "Uninstall failed".to_string(),
                };
            }
        }
    }

    reload_manager_and_notify(state);

    log::info!("Plugin {} uninstalled successfully", id);
    let message = match (removed_installed_copy, unlinked_dev) {
        (true, true) => "Uninstalled and unlinked successfully".to_string(),
        (true, false) => "Uninstalled successfully".to_string(),
        (false, true) => "Unlinked successfully".to_string(),
        (false, false) => "Uninstall completed".to_string(),
    };

    UninstallResult {
        success: true,
        message,
    }
}

fn check_update(
    cached_versions: &std::collections::HashMap<String, String>,
    id: &str,
    installed_version: &str,
) -> (Option<String>, bool) {
    let available = cached_versions.get(id).cloned();
    let update_available = available
        .as_ref()
        .map(|v| installed_version != "unknown" && is_newer_version(v, installed_version))
        .unwrap_or(false);
    (available, update_available)
}

pub(super) fn list_installed(state: &AppState) -> Result<InstalledPluginsResponse, StatusCode> {
    use super::super::github::read_cache;
    use std::collections::HashMap;

    let revision = state.daemon.events.plugins_revision();

    let manager = state.plugin_manager.lock().map_err(|error| {
        log::error!("Plugin manager mutex poisoned: {}", error);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let cached_versions: HashMap<String, String> = read_cache()
        .map(|cache| cache.plugins.into_iter().map(|plugin| (plugin.id, plugin.version)).collect())
        .unwrap_or_default();

    let mut plugins_by_id: HashMap<String, InstalledPlugin> = manager
        .plugins()
        .map(|plugin| {
            let cover_path = plugin.path.join("cover.png");
            let ui_path = plugin.path.join("ui").join("index.html");
            let (available_version, update_available) =
                check_update(&cached_versions, &plugin.id, &plugin.manifest.plugin.version);

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
            Some(manifest) => (
                manifest.plugin.name.clone(),
                manifest.plugin.description.clone(),
                manifest.plugin.version.clone(),
                extract_actions(&manifest.menu.items),
            ),
            None => (
                id.clone(),
                "Plugin manifest could not be parsed".to_string(),
                "unknown".to_string(),
                Vec::new(),
            ),
        };

        let (available_version, update_available) = check_update(&cached_versions, &id, &version);

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

    Ok(InstalledPluginsResponse { revision, plugins })
}
