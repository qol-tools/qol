use crate::plugins::paths as plugin_paths;
use crate::plugins::resolver::{
    PluginSource, PluginUnavailable, ResolutionOrigin, ResolutionReport, ResolvedPlugin,
};
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
    let report = manager.last_resolution_report().clone();
    let mut plugins_by_id = loaded_plugins_by_id(&manager, &report, &cached_versions);
    drop(manager);
    add_unavailable_plugins(&report, &cached_versions, &mut plugins_by_id);
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
    report: &ResolutionReport,
    cached_versions: &HashMap<String, String>,
) -> HashMap<PluginId, InstalledPlugin> {
    manager
        .plugins()
        .map(|plugin| {
            let resolution = find_resolved(report, &plugin.id);
            (
                plugin.id.clone(),
                loaded_plugin_info(plugin, resolution, cached_versions),
            )
        })
        .collect()
}

fn find_resolved<'a>(report: &'a ResolutionReport, id: &PluginId) -> Option<&'a ResolvedPlugin> {
    report.plugins.iter().find(|p| p.id == *id)
}

fn loaded_plugin_info(
    plugin: &crate::plugins::Plugin,
    resolution: Option<&ResolvedPlugin>,
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
        source: resolution.map(|r| source_label(&r.source)),
        resolved_from: resolution.map(|r| origin_label(r.resolved_from)),
        active_failure_reason: resolution
            .and_then(|r| r.active_failure.as_ref().map(|f| f.reason.clone())),
        unavailable: false,
    }
}

fn source_label(source: &PluginSource) -> &'static str {
    match source {
        PluginSource::Installed => "installed",
        PluginSource::DevLinked => "dev_linked",
    }
}

fn origin_label(origin: ResolutionOrigin) -> &'static str {
    match origin {
        ResolutionOrigin::Active => "active",
        ResolutionOrigin::Fallback => "fallback",
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
        source: None,
        resolved_from: None,
        active_failure_reason: None,
        unavailable: false,
    }
}

fn add_unavailable_plugins(
    report: &ResolutionReport,
    cached_versions: &HashMap<String, String>,
    plugins_by_id: &mut HashMap<PluginId, InstalledPlugin>,
) {
    for entry in &report.unavailable {
        let id = PluginId::new(entry.id.clone());
        if plugins_by_id.contains_key(&id) {
            continue;
        }
        plugins_by_id.insert(id.clone(), unavailable_plugin(id, entry, cached_versions));
    }
}

fn unavailable_plugin(
    id: PluginId,
    entry: &PluginUnavailable,
    cached_versions: &HashMap<String, String>,
) -> InstalledPlugin {
    let manifest = read_manifest_without_validation(&entry.active.path).or_else(|| {
        entry
            .fallback
            .as_ref()
            .and_then(|f| read_manifest_without_validation(&f.path))
    });
    let (name, description, version, actions) =
        unloaded_plugin_details(id.as_str(), manifest.as_ref());
    let (available_version, update_available) =
        check_update(cached_versions, id.as_str(), &version);
    InstalledPlugin {
        id,
        name,
        description,
        version,
        loaded: false,
        load_error: Some(unavailable_reason(entry)),
        has_cover: false,
        has_custom_ui: false,
        has_config: false,
        available_version,
        update_available,
        actions,
        source: None,
        resolved_from: None,
        active_failure_reason: Some(entry.active.reason.clone()),
        unavailable: true,
    }
}

fn unavailable_reason(entry: &PluginUnavailable) -> String {
    match &entry.fallback {
        Some(fallback) => format!(
            "Active slot invalid: {} (fallback also failed: {})",
            entry.active.reason, fallback.reason
        ),
        None => format!("Active slot invalid: {}", entry.active.reason),
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
