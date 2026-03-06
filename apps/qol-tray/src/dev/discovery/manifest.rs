use serde::Deserialize;
use std::path::Path;

#[derive(Deserialize)]
struct MinimalManifest {
    plugin: MinimalPluginInfo,
}

#[derive(Deserialize)]
struct MinimalPluginInfo {
    name: String,
}

pub fn read_plugin_name(toml_path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(toml_path).ok()?;

    if let Ok(manifest) = toml::from_str::<crate::plugins::PluginManifest>(&content) {
        return Some(manifest.plugin.name);
    }

    if let Ok(minimal) = toml::from_str::<MinimalManifest>(&content) {
        return Some(minimal.plugin.name);
    }

    None
}
