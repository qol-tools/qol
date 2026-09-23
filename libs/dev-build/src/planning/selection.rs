use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::platform::{BuildPlatform, Platform, PluginSupport};

pub(super) struct SelectedPlugin {
    pub(super) plugin_id: String,
    pub(super) path: PathBuf,
    pub(super) has_cargo: bool,
    pub(super) supports_platform: bool,
    pub(super) platform_reason: String,
}

pub(super) fn select_linked_plugins(dev_links: &HashMap<String, PathBuf>) -> Vec<SelectedPlugin> {
    sorted_links(dev_links)
        .into_iter()
        .map(|(plugin_id, path)| select_plugin(plugin_id, path))
        .collect()
}

fn sorted_links(dev_links: &HashMap<String, PathBuf>) -> Vec<(&String, &PathBuf)> {
    let mut links: Vec<_> = dev_links.iter().collect();
    links.sort_by_key(|(plugin_id, _)| *plugin_id);
    links
}

fn select_plugin(plugin_id: &str, path: &Path) -> SelectedPlugin {
    let platform = check_plugin_platform(path);
    SelectedPlugin {
        plugin_id: plugin_id.to_string(),
        path: path.to_path_buf(),
        has_cargo: path.join("Cargo.toml").is_file(),
        supports_platform: platform.supported,
        platform_reason: platform.reason,
    }
}

fn check_plugin_platform(path: &Path) -> PluginSupport {
    match load_manifest(path) {
        ManifestLoad::Ok(manifest) => Platform.plugin_support(&manifest),
        ManifestLoad::Missing => missing_manifest(),
        ManifestLoad::Unparseable(error) => unparseable_manifest(error),
    }
}

enum ManifestLoad {
    Ok(Box<qol_plugin_api::manifest::PluginManifest>),
    Missing,
    Unparseable(String),
}

fn load_manifest(path: &Path) -> ManifestLoad {
    let manifest_path = path.join("plugin.toml");
    let content = match std::fs::read_to_string(&manifest_path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return ManifestLoad::Missing;
        }
        Err(error) => return ManifestLoad::Unparseable(error.to_string()),
    };
    match toml::from_str::<qol_plugin_api::manifest::PluginManifest>(&content) {
        Ok(manifest) => ManifestLoad::Ok(Box::new(manifest)),
        Err(error) => ManifestLoad::Unparseable(error.to_string()),
    }
}

fn missing_manifest() -> PluginSupport {
    PluginSupport {
        supported: false,
        reason: "plugin.toml missing at dev-link path".to_string(),
    }
}

fn unparseable_manifest(error: String) -> PluginSupport {
    PluginSupport {
        supported: false,
        reason: format!("plugin.toml unreadable: {}", error),
    }
}
