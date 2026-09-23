use crate::version::normalize_semver_tag;
use anyhow::Result;

mod catalog_resolver;

pub(super) use catalog_resolver::build_plugin_metadata;
pub(crate) use catalog_resolver::{CachedPlugin, PluginMetadata};

pub(super) async fn manifest_from_response(
    plugin_id: &str,
    response: reqwest::Response,
) -> Result<Option<crate::plugins::PluginManifest>> {
    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !status.is_success() {
        return manifest_fetch_error(plugin_id, status, response).await;
    }

    let content = response.text().await?;
    let manifest: crate::plugins::PluginManifest = toml::from_str(&content)?;
    manifest.validate()?;
    Ok(Some(manifest))
}

async fn manifest_fetch_error(
    plugin_id: &str,
    status: reqwest::StatusCode,
    response: reqwest::Response,
) -> Result<Option<crate::plugins::PluginManifest>> {
    let body = response.text().await.unwrap_or_default();
    anyhow::bail!(
        "Failed to fetch plugin.toml for {}: {} {}",
        plugin_id,
        status,
        body
    )
}

pub(super) fn normalized_release_tag(tag_name: &str) -> Result<String> {
    normalize_semver_tag(tag_name)
        .ok_or_else(|| anyhow::anyhow!("latest release tag '{}' is not valid semver", tag_name))
}

#[cfg(test)]
mod catalog_tests;
