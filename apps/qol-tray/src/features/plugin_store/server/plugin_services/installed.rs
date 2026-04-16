use crate::plugins::paths as plugin_paths;
use crate::plugins::PluginId;
use axum::http::StatusCode;
use std::collections::HashMap;
use std::path::Path;

use super::super::helpers::{
    extract_actions, infer_load_error, is_newer_version, read_installed_plugin_dirs,
    read_manifest_without_validation,
};
use super::super::types::{AppState, InstalledPlugin, InstalledPluginsResponse, PluginAction};

pub(super) fn list_installed(state: &AppState) -> Result<InstalledPluginsResponse, StatusCode> {
    let revision = state.daemon.events.plugins_revision();
    let manager = state
        .plugin_manager
        .lock()
        .map_err(plugin_manager_lock_failed)?;
    let cached_versions = cached_versions();
    let mut plugins_by_id = loaded_plugins_by_id(&manager, &cached_versions);
    drop(manager);
    add_unloaded_plugins(&state.plugins_dir, &cached_versions, &mut plugins_by_id);
    Ok(InstalledPluginsResponse {
        revision,
        plugins: plugins_by_id.into_values().collect(),
    })
}

fn plugin_manager_lock_failed(
    error: std::sync::PoisonError<std::sync::MutexGuard<'_, crate::plugins::PluginManager>>,
) -> StatusCode {
    log::error!("Plugin manager mutex poisoned: {}", error);
    StatusCode::INTERNAL_SERVER_ERROR
}

fn cached_versions() -> HashMap<String, String> {
    use super::super::super::github::read_cache;

    read_cache()
        .map(|cache| {
            cache
                .plugins
                .into_iter()
                .map(|plugin| (plugin.id, plugin.version))
                .collect()
        })
        .unwrap_or_default()
}

fn loaded_plugins_by_id(
    manager: &crate::plugins::PluginManager,
    cached_versions: &HashMap<String, String>,
) -> HashMap<PluginId, InstalledPlugin> {
    manager
        .plugins()
        .map(|plugin| {
            (
                plugin.id.clone(),
                loaded_plugin_info(plugin, cached_versions),
            )
        })
        .collect()
}

fn loaded_plugin_info(
    plugin: &crate::plugins::Plugin,
    cached_versions: &HashMap<String, String>,
) -> InstalledPlugin {
    let (available_version, update_available) = check_update(
        cached_versions,
        plugin.id.as_str(),
        &plugin.manifest.plugin.version,
    );
    InstalledPlugin {
        id: plugin.id.clone(),
        name: plugin.manifest.plugin.name.clone(),
        description: plugin.manifest.plugin.description.clone(),
        version: plugin.manifest.plugin.version.clone(),
        loaded: true,
        load_error: None,
        has_cover: plugin.path.join("cover.png").exists(),
        has_custom_ui: plugin_paths::has_custom_ui(&plugin.path),
        has_config: plugin_paths::has_config(&plugin.path),
        available_version,
        update_available,
        actions: extract_actions(&plugin.manifest.menu.items),
    }
}

fn add_unloaded_plugins(
    plugins_dir: &Path,
    cached_versions: &HashMap<String, String>,
    plugins_by_id: &mut HashMap<PluginId, InstalledPlugin>,
) {
    for (raw_id, _plugin_dir) in read_installed_plugin_dirs(plugins_dir) {
        if plugins_by_id.contains_key(raw_id.as_str()) {
            continue;
        }
        let resolved_root =
            plugin_paths::resolve_plugin_root_from_plugins_dir(plugins_dir, &raw_id);
        let id = PluginId::new(raw_id);
        plugins_by_id.insert(
            id.clone(),
            unloaded_plugin(id, resolved_root, cached_versions),
        );
    }
}

fn unloaded_plugin(
    id: PluginId,
    plugin_dir: std::path::PathBuf,
    cached_versions: &HashMap<String, String>,
) -> InstalledPlugin {
    let manifest = read_manifest_without_validation(&plugin_dir);
    let (name, description, version, actions) =
        unloaded_plugin_details(id.as_str(), manifest.as_ref());
    let (available_version, update_available) =
        check_update(cached_versions, id.as_str(), &version);

    let load_error = infer_load_error(id.as_str(), &plugin_dir, manifest.as_ref());
    InstalledPlugin {
        id,
        name,
        description,
        version,
        loaded: false,
        load_error,
        has_cover: plugin_dir.join("cover.png").exists(),
        has_custom_ui: plugin_paths::has_custom_ui(&plugin_dir),
        has_config: plugin_paths::has_config(&plugin_dir),
        available_version,
        update_available,
        actions,
    }
}

fn unloaded_plugin_details(
    id: &str,
    manifest: Option<&crate::plugins::PluginManifest>,
) -> (String, String, String, Vec<PluginAction>) {
    let manifest = match manifest {
        Some(manifest) => manifest,
        None => {
            return (
                id.to_string(),
                "Plugin manifest could not be parsed".to_string(),
                "unknown".to_string(),
                Vec::new(),
            )
        }
    };
    (
        manifest.plugin.name.clone(),
        manifest.plugin.description.clone(),
        manifest.plugin.version.clone(),
        extract_actions(&manifest.menu.items),
    )
}

fn check_update(
    cached_versions: &HashMap<String, String>,
    id: &str,
    installed_version: &str,
) -> (Option<String>, bool) {
    let available = cached_versions.get(id).cloned();
    let update_available = available
        .as_ref()
        .map(|version| {
            installed_version != "unknown" && is_newer_version(version, installed_version)
        })
        .unwrap_or(false);
    (available, update_available)
}
