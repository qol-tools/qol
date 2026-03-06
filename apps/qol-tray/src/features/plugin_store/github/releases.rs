use super::super::release_assets::PlatformTarget;
use super::catalog::{normalized_release_tag, required_release_assets};
use super::GitHubClient;
use anyhow::Result;
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize)]
pub(super) struct GitHubRelease {
    tag_name: String,
    assets: Vec<GitHubAsset>,
}

#[derive(Debug, Clone, Deserialize)]
struct GitHubAsset {
    name: String,
}

impl GitHubClient {
    pub(super) async fn fetch_release_version(
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

    async fn verify_platform_binary_support(&self, manifest: &crate::plugins::PluginManifest, target: PlatformTarget, plugin_repo: &str, plugin_release: &GitHubRelease) -> Result<()> {
        let required_assets = required_release_assets(manifest, target)?;
        let mut release_cache = HashMap::from([(plugin_repo.to_string(), plugin_release.clone())]);
        for required_asset in required_assets {
            let release = self.release_for_repo(&mut release_cache, &required_asset.repo).await?;
            if !release_has_asset(release, &required_asset.name) {
                anyhow::bail!("missing asset '{}' in latest release of {}", required_asset.name, required_asset.repo);
            }
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
        let response = super::send_checked(self.build_request(&url)).await?;
        Ok(response.json().await?)
    }
}

fn release_has_asset(release: &GitHubRelease, asset_name: &str) -> bool {
    release.assets.iter().any(|asset| asset.name == asset_name)
}
