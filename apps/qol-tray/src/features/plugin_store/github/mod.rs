mod cache;
mod token;

pub use cache::{
    cache_age_secs, read_cache, update_cached_version, write_cache, CachedPlugin, PluginCache,
};
pub use token::{
    build_github_request, delete_token, get_stored_token, send_checked, store_token,
    validate_token, TokenValidationError,
};

use super::release_assets::{resolve_asset_pattern, PlatformTarget};
use crate::version::normalize_semver_tag;
use anyhow::Result;
use serde::Deserialize;
use std::collections::HashMap;

const PLUGIN_PREFIX: &str = "plugin-";
const CACHE_TTL_SECS: u64 = 3600;
const CACHE_FORMAT_VERSION: u32 = 2;

#[derive(Debug, Deserialize)]
struct GitHubRepo {
    name: String,
    html_url: String,
}

#[derive(Debug, Clone, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    assets: Vec<GitHubAsset>,
}

#[derive(Debug, Clone, Deserialize)]
struct GitHubAsset {
    name: String,
}

#[derive(Debug, Clone)]
struct RequiredReleaseAsset {
    repo: String,
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
        normalized_release_tag(&plugin_release)
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

fn manifest_url(org: &str, repo_name: &str, branch: &str) -> String {
    format!(
        "https://raw.githubusercontent.com/{}/{}/{}/plugin.toml",
        org, repo_name, branch
    )
}

async fn manifest_from_response(
    repo_name: &str,
    branch: &str,
    response: reqwest::Response,
) -> Result<Option<crate::plugins::PluginManifest>> {
    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !status.is_success() {
        return manifest_fetch_error(repo_name, branch, status, response).await;
    }

    let content = response.text().await?;
    let manifest: crate::plugins::PluginManifest = toml::from_str(&content)?;
    manifest.validate()?;
    Ok(Some(manifest))
}

async fn manifest_fetch_error(
    repo_name: &str,
    branch: &str,
    status: reqwest::StatusCode,
    response: reqwest::Response,
) -> Result<Option<crate::plugins::PluginManifest>> {
    let body = response.text().await.unwrap_or_default();
    anyhow::bail!(
        "Failed to fetch plugin.toml for {} on {}: {} {}",
        repo_name,
        branch,
        status,
        body
    )
}

fn normalized_release_tag(release: &GitHubRelease) -> Result<String> {
    normalize_semver_tag(&release.tag_name).ok_or_else(|| {
        anyhow::anyhow!(
            "latest release tag '{}' is not valid semver",
            release.tag_name
        )
    })
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

fn is_plugin_repo(name: &str) -> bool {
    if name.trim() != name || name == "plugin-template" {
        return false;
    }
    let Some(suffix) = name.strip_prefix(PLUGIN_PREFIX) else {
        return false;
    };
    !suffix.is_empty()
        && !suffix.starts_with('-')
        && !suffix.ends_with('-')
        && suffix
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
}

fn filter_plugin_repos(repos: &[GitHubRepo]) -> Vec<&GitHubRepo> {
    repos
        .iter()
        .filter(|repo| is_plugin_repo(&repo.name))
        .collect()
}

fn build_plugin_metadata(
    repo: &GitHubRepo,
    manifest: crate::plugins::PluginManifest,
    version: String,
) -> PluginMetadata {
    PluginMetadata {
        id: repo.name.clone(),
        name: manifest.plugin.name,
        description: manifest.plugin.description,
        version,
        repo_url: repo.html_url.clone(),
        platforms: manifest.plugin.platforms,
    }
}

fn required_release_assets(
    manifest: &crate::plugins::PluginManifest,
    target: PlatformTarget,
) -> Result<Vec<RequiredReleaseAsset>> {
    let dependencies = manifest
        .dependencies
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("manifest is missing dependencies.binaries"))?;
    if dependencies.binaries.is_empty() {
        anyhow::bail!("manifest has empty dependencies.binaries");
    }
    Ok(dependencies
        .binaries
        .iter()
        .map(|binary| RequiredReleaseAsset {
            repo: binary.repo.clone(),
            name: resolve_asset_pattern(&binary.pattern, target),
        })
        .collect())
}

#[derive(Debug, Clone, PartialEq)]
pub struct PluginMetadata {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub repo_url: String,
    pub platforms: Option<Vec<String>>,
}

impl PluginMetadata {
    pub fn supports_current_platform(&self) -> bool {
        crate::plugins::manifest::supports_current_platform(&self.platforms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::manifest::{MenuConfig, PluginInfo, PluginManifest};

    fn make_repo(name: &str) -> GitHubRepo {
        GitHubRepo {
            name: name.to_string(),
            html_url: format!("https://github.com/test/{}", name),
        }
    }

    fn make_manifest(name: &str, version: &str) -> PluginManifest {
        PluginManifest {
            manifest_version: crate::plugins::manifest::CURRENT_MANIFEST_VERSION,
            plugin: PluginInfo {
                name: name.to_string(),
                description: "Test plugin".to_string(),
                version: version.to_string(),
                author: None,
                platforms: None,
            },
            menu: MenuConfig {
                label: "Test".to_string(),
                icon: None,
                items: vec![],
            },
            daemon: None,
            dependencies: None,
            runtime: None,
        }
    }

    #[test]
    fn is_plugin_repo_filtering() {
        let cases = [
            ("plugin-screen-recorder", true),
            ("plugin-notes", true),
            ("plugin-a", true),
            ("plugin-123", true),
            ("plugin-caps", true),
            ("screen-recorder", false),
            ("my-plugin", false),
            ("pluginstore", false),
            ("plugin-template", false),
            ("", false),
            ("plugin-", false),
            ("plugin", false),
            ("PLUGIN-foo", false),
            ("Plugin-foo", false),
            (" plugin-foo", false),
            ("plugin-foo ", false),
            ("plugin--double", false),
        ];

        for (name, expected) in cases {
            assert_eq!(is_plugin_repo(name), expected, "name: {:?}", name);
        }
    }

    #[test]
    fn filter_plugin_repos_selects_valid_plugins() {
        let cases = [
            (
                vec!["plugin-recorder", "some-tool", "plugin-notes", "pluginish"],
                vec!["plugin-recorder", "plugin-notes"],
            ),
            (vec!["tool-one", "tool-two"], vec![]),
        ];

        for (input_names, expected_names) in cases {
            let repos: Vec<_> = input_names.iter().map(|name| make_repo(name)).collect();
            let filtered = filter_plugin_repos(&repos);
            let names: Vec<_> = filtered.iter().map(|repo| repo.name.as_str()).collect();
            assert_eq!(names, expected_names, "input: {:?}", input_names);
        }
    }

    #[test]
    fn build_plugin_metadata_uses_provided_version() {
        let repo = make_repo("plugin-test");
        let manifest = make_manifest("Test", "1.0.0");
        let metadata = build_plugin_metadata(&repo, manifest, "1.0.0".to_string());
        assert_eq!(metadata.version, "1.0.0");
    }

    #[test]
    fn build_plugin_metadata_extracts_all_fields() {
        let repo = make_repo("plugin-example");
        let manifest = make_manifest("Example Plugin", "2.5.0");
        let metadata = build_plugin_metadata(&repo, manifest, "3.0.0".to_string());

        assert_eq!(metadata.id, "plugin-example");
        assert_eq!(metadata.name, "Example Plugin");
        assert_eq!(metadata.description, "Test plugin");
        assert_eq!(metadata.version, "3.0.0");
        assert_eq!(metadata.repo_url, "https://github.com/test/plugin-example");
    }

    fn make_metadata(platforms: Option<Vec<&str>>) -> PluginMetadata {
        PluginMetadata {
            id: "test".to_string(),
            name: "Test".to_string(),
            description: "Test".to_string(),
            version: "1.0.0".to_string(),
            repo_url: "https://example.com".to_string(),
            platforms: platforms.map(|platforms| platforms.into_iter().map(String::from).collect()),
        }
    }

    #[test]
    fn plugin_metadata_supports_current_platform_cases() {
        let current_os = std::env::consts::OS;
        let cases: &[(Option<Vec<&str>>, bool)] = &[
            (None, true),
            (Some(vec![]), false),
            (Some(vec![current_os]), true),
            (Some(vec!["not-a-real-os"]), false),
            (Some(vec!["linux", "windows", "macos"]), true),
            (Some(vec!["fake1", "fake2"]), false),
            (Some(vec!["LINUX"]), false),
        ];

        for (platforms, expected) in cases {
            let metadata = make_metadata(platforms.clone());
            assert_eq!(
                metadata.supports_current_platform(),
                *expected,
                "platforms: {:?}",
                platforms
            );
        }
    }

    #[test]
    fn cached_plugin_roundtrip() {
        let metadata = PluginMetadata {
            id: "plugin-test".to_string(),
            name: "Test Plugin".to_string(),
            description: "A test".to_string(),
            version: "1.2.3".to_string(),
            repo_url: "https://github.com/test/plugin-test".to_string(),
            platforms: Some(vec!["linux".to_string()]),
        };

        let cached: CachedPlugin = metadata.clone().into();
        let back: PluginMetadata = cached.into();

        assert_eq!(back.id, metadata.id);
        assert_eq!(back.name, metadata.name);
        assert_eq!(back.description, metadata.description);
        assert_eq!(back.version, metadata.version);
        assert_eq!(back.repo_url, metadata.repo_url);
        assert_eq!(back.platforms, metadata.platforms);
    }
}
