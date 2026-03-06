use axum::http::StatusCode;
use std::collections::HashMap;
use std::path::Path;

use crate::plugins::PluginLoader;

use super::super::helpers::{read_installed_plugin_dirs, read_plugin_version};
use super::super::types::{PluginInfo, PluginsResponse};

pub(super) async fn list_plugins(refresh: bool) -> Result<PluginsResponse, (StatusCode, String)> {
    use super::super::super::github::{cache_age_secs, GitHubClient};

    log::info!("API /plugins called (refresh={})", refresh);
    let client = GitHubClient::new("qol-tools");
    let plugins_dir = plugins_dir()?;
    let installed_versions = installed_versions(&plugins_dir);
    let cache_age = cache_age_secs();
    let metadata_list = fetch_metadata(&client, refresh).await?;

    log::info!("Got {} plugins", metadata_list.len());
    Ok(PluginsResponse {
        plugins: metadata_list
            .into_iter()
            .filter(|metadata| metadata.supports_current_platform())
            .map(|metadata| plugin_info(metadata, &installed_versions))
            .collect(),
        cache_age_secs: cache_age,
    })
}

fn plugins_dir() -> Result<std::path::PathBuf, (StatusCode, String)> {
    PluginLoader::default_plugin_dir().map_err(|error| {
        log::error!("Failed to determine config directory: {}", error);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to determine plugin directory".to_string(),
        )
    })
}

fn installed_versions(plugins_dir: &Path) -> HashMap<String, String> {
    read_installed_plugin_dirs(plugins_dir)
        .into_iter()
        .filter_map(|(id, path)| read_plugin_version(&path).ok().map(|version| (id, version)))
        .collect()
}

async fn fetch_metadata(
    client: &super::super::super::github::GitHubClient,
    refresh: bool,
) -> Result<Vec<super::super::super::github::PluginMetadata>, (StatusCode, String)> {
    client.list_plugins_cached(refresh).await.map_err(|error| {
        log::error!("Failed to fetch plugins: {}", error);
        (
            StatusCode::SERVICE_UNAVAILABLE,
            format!("Failed to fetch plugins: {:#}", error),
        )
    })
}

fn plugin_info(
    metadata: super::super::super::github::PluginMetadata,
    installed_versions: &HashMap<String, String>,
) -> PluginInfo {
    let installed_version = installed_versions.get(&metadata.id).cloned();
    PluginInfo {
        id: metadata.id.clone(),
        name: metadata.name,
        description: metadata.description,
        version: metadata.version,
        installed: installed_version.is_some(),
        installed_version,
    }
}
