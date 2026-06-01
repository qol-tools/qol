mod cache;
mod catalog;
mod manifests;
mod releases;
mod token;

#[cfg(test)]
pub(crate) use cache::CachedPlugin;
pub(crate) use cache::{
    current_timestamp, read_cache, update_cached_version, write_cache, PluginCache,
};
pub(crate) use catalog::PluginMetadata;
pub(crate) use token::{build_github_request, get_stored_token, send_checked};

use anyhow::Result;
use catalog::{build_plugin_metadata, filter_plugin_repos, GitHubRepo};

pub(crate) const CACHE_TTL_SECS: u64 = 3600;
pub(crate) const CACHE_FORMAT_VERSION: u32 = 2;

pub(crate) struct GitHubClient {
    org: String,
    client: reqwest::Client,
    token: Option<String>,
}

impl GitHubClient {
    pub(crate) fn new(org: impl Into<String>) -> Self {
        Self {
            org: org.into(),
            client: reqwest::Client::new(),
            token: get_stored_token(),
        }
    }

    fn build_request(&self, url: &str) -> reqwest::RequestBuilder {
        build_github_request(&self.client, url, self.token.as_deref())
    }

    pub(crate) async fn list_plugins(&self) -> Result<Vec<PluginMetadata>> {
        let repos = self.fetch_plugin_repos().await?;
        let mut plugins = Vec::new();

        for repo in repos {
            let Some(metadata) = self.plugin_metadata(&repo).await else {
                continue;
            };
            plugins.push(metadata);
        }

        Ok(plugins)
    }

    async fn fetch_plugin_repos(&self) -> Result<Vec<GitHubRepo>> {
        let url = format!("https://api.github.com/orgs/{}/repos", self.org);
        let response = send_checked(self.build_request(&url)).await?;
        let repos: Vec<GitHubRepo> = response.json().await?;
        Ok(filter_plugin_repos(&repos).into_iter().cloned().collect())
    }

    async fn plugin_metadata(&self, repo: &GitHubRepo) -> Option<PluginMetadata> {
        let manifest = match self.fetch_plugin_manifest(&repo.name).await {
            Ok(manifest) => manifest,
            Err(error) => return skip_repo(&repo.name, &error),
        };
        let version = match self.fetch_release_version(&repo.name, &manifest).await {
            Ok(version) => version,
            Err(error) => return skip_release(&repo.name, &error),
        };
        Some(build_plugin_metadata(repo, manifest, version))
    }
}

fn skip_repo(repo_name: &str, error: &anyhow::Error) -> Option<PluginMetadata> {
    log::warn!("Skipping {}: {}", repo_name, error);
    None
}

fn skip_release(repo_name: &str, error: &anyhow::Error) -> Option<PluginMetadata> {
    log::warn!(
        "Skipping {}: release/binary requirements not met: {}",
        repo_name,
        error
    );
    None
}
