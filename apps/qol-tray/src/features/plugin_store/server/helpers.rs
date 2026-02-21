use crate::hotkeys::trigger_reload;
use crate::paths::is_safe_path_component;
use crate::plugins::{MenuItem, PluginLoader, PluginManifest};

use super::types::{AppState, PluginAction};

pub(super) fn read_installed_plugin_dirs(
    plugins_dir: &std::path::Path,
) -> Vec<(String, std::path::PathBuf)> {
    if !plugins_dir.exists() {
        return Vec::new();
    }

    std::fs::read_dir(plugins_dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter_map(|entry| {
            let path = entry.path();
            let metadata = std::fs::symlink_metadata(&path).ok()?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return None;
            }
            let id = entry.file_name().into_string().ok()?;
            if id.starts_with('.') || id.ends_with(".backup") || !is_safe_path_component(&id) {
                return None;
            }
            Some((id, path))
        })
        .collect()
}

pub(super) fn read_plugin_version(plugin_dir: &std::path::Path) -> Result<String, ()> {
    let manifest_path = plugin_dir.join("plugin.toml");
    let content = std::fs::read_to_string(&manifest_path).map_err(|_| ())?;
    let manifest: PluginManifest = toml::from_str(&content).map_err(|_| ())?;
    Ok(manifest.plugin.version)
}

pub(super) fn reload_manager_and_notify(state: &AppState) {
    let mut manager = match state.plugin_manager.lock() {
        Ok(m) => m,
        Err(e) => {
            log::error!("Plugin manager mutex poisoned: {}", e);
            return;
        }
    };
    if let Err(e) = manager.reload_plugins() {
        log::error!("Failed to reload plugins: {}", e);
    }
    trigger_reload();
    state.daemon.events.send_plugins_changed();
}

pub(super) fn extract_actions(items: &[MenuItem]) -> Vec<PluginAction> {
    let mut actions = Vec::new();
    let mut collect = |item: &MenuItem| match item {
        MenuItem::Action {
            id, label, action, ..
        } => actions.push(PluginAction {
            id: id.clone(),
            label: label.clone(),
            kind: *action,
        }),
        MenuItem::Checkbox { .. } | MenuItem::Separator | MenuItem::Submenu { .. } => {}
    };
    crate::plugins::manifest::walk_menu_items(items, &mut collect);
    actions
}

pub(super) fn is_newer_version(available: &str, installed: &str) -> bool {
    use crate::version::Version;
    Version::parse(available).is_newer_than(&Version::parse(installed))
}

pub(super) fn read_manifest_without_validation(
    plugin_dir: &std::path::Path,
) -> Option<PluginManifest> {
    let manifest_path = plugin_dir.join("plugin.toml");
    let content = std::fs::read_to_string(manifest_path).ok()?;
    toml::from_str(&content).ok()
}

pub(super) fn infer_load_error(
    plugin_id: &str,
    plugin_dir: &std::path::Path,
    manifest: Option<&PluginManifest>,
) -> Option<String> {
    let Some(manifest) = manifest else {
        return Some("Invalid plugin.toml".to_string());
    };

    if !manifest.plugin.supports_current_platform() {
        return Some(format!(
            "Unsupported platform: current platform is {}",
            std::env::consts::OS
        ));
    }

    PluginLoader::load_plugin_with_id(plugin_id, plugin_dir)
        .err()
        .map(|e| format!("{:#}", e))
}
