use super::catalog::manifest_from_response;
use super::GitHubClient;
use anyhow::Result;

impl GitHubClient {
    pub(super) async fn fetch_plugin_manifest(
        &self,
        plugin_id: &str,
    ) -> Result<crate::plugins::PluginManifest> {
        let url = self.source.manifest_raw_url(plugin_id);
        let response = self.build_request(&url).send().await?;
        let Some(manifest) = manifest_from_response(plugin_id, response).await? else {
            anyhow::bail!("plugin.toml not found at {} for {}", url, plugin_id);
        };
        Ok(manifest)
    }
}
