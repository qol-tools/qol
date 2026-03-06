use super::catalog::{manifest_from_response, manifest_url};
use super::GitHubClient;
use anyhow::Result;

impl GitHubClient {
    pub(super) async fn fetch_plugin_manifest(
        &self,
        repo_name: &str,
    ) -> Result<crate::plugins::PluginManifest> {
        for branch in ["main", "master"] {
            let response = self.fetch_manifest_response(repo_name, branch).await?;
            let Some(manifest) = manifest_from_response(repo_name, branch, response).await? else {
                continue;
            };
            return Ok(manifest);
        }

        anyhow::bail!(
            "plugin.toml not found for {} on main or master branch",
            repo_name
        )
    }

    async fn fetch_manifest_response(
        &self,
        repo_name: &str,
        branch: &str,
    ) -> Result<reqwest::Response> {
        let url = manifest_url(&self.org, repo_name, branch);
        Ok(self.build_request(&url).send().await?)
    }
}
