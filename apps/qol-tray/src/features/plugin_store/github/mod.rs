mod cache;
mod catalog;
mod token;

pub use cache::{
    cache_age_secs, read_cache, update_cached_version, write_cache, CachedPlugin, PluginCache,
};
pub use catalog::PluginMetadata;
pub use token::{
    build_github_request, delete_token, get_stored_token, send_checked, store_token,
    validate_token, TokenValidationError,
};

use super::release_assets::PlatformTarget;
use anyhow::Result;
use serde::Deserialize;
use std::collections::HashMap;

const CACHE_TTL_SECS: u64 = 3600;
const CACHE_FORMAT_VERSION: u32 = 2;
use catalog::{
    build_plugin_metadata, filter_plugin_repos, manifest_from_response, manifest_url,
    normalized_release_tag, required_release_assets, GitHubRepo,
};

#[derive(Debug, Clone, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    assets: Vec<GitHubAsset>,
}

#[derive(Debug, Clone, Deserialize)]
struct GitHubAsset {
    name: String,
}

pub struct GitHubClient {
    org: String,
    client: reqwest::Client,
    token: Option<String>,
}

impl GitHubClient {
    pub fn new(org: impl Into<String>) -> Self {
        Self {
            org: org.into(),
            client: reqwest::Client::new(),
            token: get_stored_token(),
        }
    }

    fn build_request(&self, url: &str) -> reqwest::RequestBuilder {
        build_github_request(&self.client, url, self.token.as_deref())
    }

    pub async fn list_plugins(&self) -> Result<Vec<PluginMetadata>> {
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

    async fn fetch_plugin_manifest(
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

    async fn fetch_release_version(
        &self,
        repo_name: &str,
        manifest: &crate::plugins::PluginManifest,
    ) -> Result<String> {
        let target = PlatformTarget::current()?;
        let plugin_repo = format!("{}/{}", self.org, repo_name);
        let plugin_release = self.fetch_latest_release(&plugin_repo).await?;
        self.verify_platform_binary_support(manifest, target, &plugin_repo, &plugin_release)
            .await?;
        normalized_release_tag(&plugin_release.tag_name)
    }

    async fn verify_platform_binary_support(
        &self,
        manifest: &crate::plugins::PluginManifest,
        target: PlatformTarget,
        plugin_repo: &str,
        plugin_release: &GitHubRelease,
    ) -> Result<()> {
        let required_assets = required_release_assets(manifest, target)?;
        let mut release_cache = HashMap::from([(plugin_repo.to_string(), plugin_release.clone())]);

        for required_asset in required_assets {
            let release = self
                .release_for_repo(&mut release_cache, &required_asset.repo)
                .await?;
            if release_has_asset(release, &required_asset.name) {
                continue;
            }
            anyhow::bail!(
                "missing asset '{}' in latest release of {}",
                required_asset.name,
                required_asset.repo
            );
        }

        Ok(())
    }

    async fn release_for_repo<'a>(
        &self,
        release_cache: &'a mut HashMap<String, GitHubRelease>,
        repo: &str,
    ) -> Result<&'a GitHubRelease> {
        if !release_cache.contains_key(repo) {
            let release = self.fetch_latest_release(repo).await?;
            release_cache.insert(repo.to_string(), release);
        }
        Ok(release_cache
            .get(repo)
            .expect("release cache entry inserted"))
    }

    async fn fetch_latest_release(&self, repo: &str) -> Result<GitHubRelease> {
        let url = format!("https://api.github.com/repos/{}/releases/latest", repo);
        let response = send_checked(self.build_request(&url)).await?;
        Ok(response.json().await?)
    }

    pub async fn list_plugins_cached(&self, force_refresh: bool) -> Result<Vec<PluginMetadata>> {
        if !force_refresh {
            if let Some(plugins) = cache::valid_cache() {
                return Ok(plugins);
            }
        }

        log::info!("Fetching fresh plugin data from GitHub");
        let plugins = self.list_plugins().await?;
        if let Err(error) = write_cache(&plugins) {
            log::warn!("Failed to write plugin cache: {}", error);
        }
        Ok(plugins)
    }
}

fn release_has_asset(release: &GitHubRelease, asset_name: &str) -> bool {
    release.assets.iter().any(|asset| asset.name == asset_name)
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
