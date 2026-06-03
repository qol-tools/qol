mod cache;
mod catalog;
mod manifests;
mod releases;
mod token;
mod tree;

#[cfg(test)]
pub(crate) use cache::CachedPlugin;
pub(crate) use cache::{
    current_timestamp, read_cache, update_cached_version, write_cache, PluginCache,
};
pub(crate) use catalog::PluginMetadata;
pub(crate) use token::{build_github_request, get_stored_token, send_checked};

use super::source::PluginSource;
use anyhow::Result;
use catalog::build_plugin_metadata;
use tree::collect_plugin_ids;

pub(crate) const CACHE_TTL_SECS: u64 = 3600;
pub(crate) const CACHE_FORMAT_VERSION: u32 = 2;

pub(crate) struct GitHubClient {
    source: PluginSource,
    client: reqwest::Client,
    token: Option<String>,
}

impl GitHubClient {
    pub(crate) fn new(source: PluginSource) -> Self {
        Self {
            source,
            client: reqwest::Client::new(),
            token: get_stored_token(),
        }
    }

    fn build_request(&self, url: &str) -> reqwest::RequestBuilder {
        build_github_request(&self.client, url, self.token.as_deref())
    }

    pub(crate) async fn list_plugins(&self) -> Result<Vec<PluginMetadata>> {
        let plugin_ids = self.fetch_plugin_ids().await?;
        if plugin_ids.is_empty() {
            log::info!(
                "Plugin source {} ({}) tree contains no plugins/<id>/plugin.toml entries",
                self.source.name,
                self.source.repo
            );
            return Ok(Vec::new());
        }
        let releases = match self.fetch_source_releases().await {
            Ok(releases) => releases,
            Err(error) => {
                log::warn!(
                    "Failed to list releases for source {} ({}): {:#}",
                    self.source.name,
                    self.source.repo,
                    error
                );
                return Ok(Vec::new());
            }
        };

        let mut plugins = Vec::new();
        for plugin_id in plugin_ids {
            let Some(metadata) = self.plugin_metadata(&plugin_id, &releases).await else {
                continue;
            };
            plugins.push(metadata);
        }
        Ok(plugins)
    }

    async fn fetch_plugin_ids(&self) -> Result<Vec<String>> {
        let url = self.source.tree_api_url();
        let response = send_checked(self.build_request(&url)).await?;
        let tree: tree::TreeResponse = response.json().await?;
        Ok(collect_plugin_ids(&tree))
    }

    async fn plugin_metadata(
        &self,
        plugin_id: &str,
        releases: &[releases::GitHubRelease],
    ) -> Option<PluginMetadata> {
        let manifest = match self.fetch_plugin_manifest(plugin_id).await {
            Ok(manifest) => manifest,
            Err(error) => return skip_plugin_manifest(plugin_id, &error),
        };
        let version = match self.select_plugin_version(plugin_id, releases, &manifest) {
            Ok(version) => version,
            Err(error) => return skip_plugin_release(plugin_id, &error),
        };
        Some(build_plugin_metadata(
            plugin_id,
            &self.source,
            manifest,
            version,
        ))
    }
}

fn skip_plugin_manifest(plugin_id: &str, error: &anyhow::Error) -> Option<PluginMetadata> {
    log::warn!("Skipping {}: manifest fetch failed: {}", plugin_id, error);
    None
}

fn skip_plugin_release(plugin_id: &str, error: &anyhow::Error) -> Option<PluginMetadata> {
    log::warn!(
        "Skipping {}: release/binary requirements not met: {}",
        plugin_id,
        error
    );
    None
}
