use super::release_assets::{PlatformTarget, resolve_asset_pattern};
use crate::{paths, version::normalize_semver_tag};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const PLUGIN_PREFIX: &str = "plugin-";
const CACHE_TTL_SECS: u64 = 3600;
const CACHE_FORMAT_VERSION: u32 = 2;

fn token_path() -> Option<PathBuf> {
    paths::github_token_path().ok()
}

fn cache_path() -> Option<PathBuf> {
    paths::plugin_cache_path().ok()
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PluginCache {
    #[serde(default)]
    pub format_version: u32,
    pub timestamp: u64,
    pub plugins: Vec<CachedPlugin>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedPlugin {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub repo_url: String,
    #[serde(default)]
    pub platforms: Option<Vec<String>>,
}

impl From<PluginMetadata> for CachedPlugin {
    fn from(m: PluginMetadata) -> Self {
        Self {
            id: m.id,
            name: m.name,
            description: m.description,
            version: m.version,
            repo_url: m.repo_url,
            platforms: m.platforms,
        }
    }
}

impl From<CachedPlugin> for PluginMetadata {
    fn from(c: CachedPlugin) -> Self {
        Self {
            id: c.id,
            name: c.name,
            description: c.description,
            version: c.version,
            repo_url: c.repo_url,
            platforms: c.platforms,
        }
    }
}

fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn read_cache() -> Option<PluginCache> {
    let path = cache_path()?;
    let content = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&content).ok()
}

pub fn write_cache(plugins: &[PluginMetadata]) -> Result<()> {
    let Some(path) = cache_path() else {
        anyhow::bail!("Could not determine cache path");
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let cache = PluginCache {
        format_version: CACHE_FORMAT_VERSION,
        timestamp: current_timestamp(),
        plugins: plugins.iter().cloned().map(CachedPlugin::from).collect(),
    };
    let content = serde_json::to_string(&cache)?;
    std::fs::write(&path, content)?;
    log::info!("Plugin cache written to {:?}", path);
    Ok(())
}

pub fn cache_age_secs() -> Option<u64> {
    read_cache().map(|c| current_timestamp() - c.timestamp)
}

pub fn update_cached_version(plugin_id: &str, version: &str) {
    let Some(mut cache) = read_cache() else {
        return;
    };
    let Some(plugin) = cache.plugins.iter_mut().find(|p| p.id == plugin_id) else {
        return;
    };
    let Some(path) = cache_path() else { return };

    plugin.version = version.to_string();
    let Ok(content) = serde_json::to_string(&cache) else {
        return;
    };
    let _ = std::fs::write(path, content);
    log::info!("Updated cache version for {}: {}", plugin_id, version);
}

fn get_valid_cache() -> Option<Vec<PluginMetadata>> {
    let cache = read_cache()?;
    if cache.format_version != CACHE_FORMAT_VERSION {
        return None;
    }
    let age = current_timestamp() - cache.timestamp;

    if age >= CACHE_TTL_SECS {
        return None;
    }

    log::info!("Using cached plugin data ({} seconds old)", age);
    Some(
        cache
            .plugins
            .into_iter()
            .map(PluginMetadata::from)
            .collect(),
    )
}

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

pub(crate) fn build_github_request(
    client: &reqwest::Client,
    url: &str,
    token: Option<&str>,
) -> reqwest::RequestBuilder {
    let mut req = client.get(url).header("User-Agent", "qol-tray");
    if let Some(token) = token {
        req = req.header("Authorization", format!("Bearer {}", token));
    }
    req
}

pub(crate) async fn send_checked(
    request: reqwest::RequestBuilder,
) -> Result<reqwest::Response> {
    let response = request.send().await?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("GitHub API returned {}: {}", status, body);
    }
    Ok(response)
}

pub fn get_stored_token() -> Option<String> {
    let path = token_path()?;

    let metadata = std::fs::symlink_metadata(&path).ok()?;
    if metadata.file_type().is_symlink() {
        log::warn!("Token file is a symlink, rejecting: {:?}", path);
        return None;
    }

    let token = std::fs::read_to_string(&path).ok()?;
    let token = token.trim();

    if token.is_empty() {
        return None;
    }

    log::info!("Loaded GitHub token from {:?}", path);
    Some(token.to_string())
}

pub fn store_token(token: &str) -> Result<()> {
    let Some(path) = token_path() else {
        anyhow::bail!("Could not determine token path");
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, token.trim())?;
    log::info!("Stored GitHub token to {:?}", path);
    Ok(())
}

#[derive(Debug)]
pub enum TokenValidationError {
    Empty,
    Invalid(String),
    Upstream(String),
}

impl std::fmt::Display for TokenValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "Token cannot be empty"),
            Self::Invalid(detail) => write!(f, "Invalid token: {}", detail),
            Self::Upstream(detail) => write!(f, "GitHub unavailable: {}", detail),
        }
    }
}

impl std::error::Error for TokenValidationError {}

pub async fn validate_token(token: &str) -> std::result::Result<(), TokenValidationError> {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return Err(TokenValidationError::Empty);
    }

    let client = reqwest::Client::new();
    let response = client
        .get("https://api.github.com/user")
        .header("User-Agent", "qol-tray")
        .header("Authorization", format!("Bearer {}", trimmed))
        .send()
        .await
        .map_err(|e| TokenValidationError::Upstream(e.to_string()))?;

    let status = response.status();
    if status.is_success() {
        return Ok(());
    }

    let body = response.text().await.unwrap_or_default();

    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(TokenValidationError::Invalid(format!("{}: {}", status, body)));
    }

    Err(TokenValidationError::Upstream(format!("{}: {}", status, body)))
}

pub fn delete_token() -> Result<()> {
    let Some(path) = token_path() else {
        return Ok(());
    };
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    Ok(())
}

impl GitHubClient {
    pub fn new(org: impl Into<String>) -> Self {
        let token = get_stored_token();
        Self {
            org: org.into(),
            client: reqwest::Client::new(),
            token,
        }
    }

    fn build_request(&self, url: &str) -> reqwest::RequestBuilder {
        build_github_request(&self.client, url, self.token.as_deref())
    }

    pub async fn list_plugins(&self) -> Result<Vec<PluginMetadata>> {
        let url = format!("https://api.github.com/orgs/{}/repos", self.org);

        let response = send_checked(self.build_request(&url)).await?;
        let repos: Vec<GitHubRepo> = response.json().await?;

        let plugin_repos = filter_plugin_repos(&repos);
        let mut plugins = Vec::new();

        for repo in plugin_repos {
            match self.fetch_plugin_manifest(&repo.name).await {
                Ok(manifest) => match self.fetch_release_version(&repo.name, &manifest).await {
                    Ok(version) => plugins.push(build_plugin_metadata(repo, manifest, version)),
                    Err(error) => {
                        log::warn!(
                            "Skipping {}: release/binary requirements not met: {}",
                            repo.name,
                            error
                        );
                    }
                },
                Err(error) => {
                    log::warn!("Skipping {}: {}", repo.name, error);
                }
            }
        }

        Ok(plugins)
    }

    async fn fetch_plugin_manifest(
        &self,
        repo_name: &str,
    ) -> Result<crate::plugins::PluginManifest> {
        for branch in ["main", "master"] {
            let url = format!(
                "https://raw.githubusercontent.com/{}/{}/{}/plugin.toml",
                self.org, repo_name, branch
            );

            let response = self.build_request(&url).send().await?;
            let status = response.status();

            if status.is_success() {
                let content = response.text().await?;
                let manifest: crate::plugins::PluginManifest = toml::from_str(&content)?;
                manifest.validate()?;
                return Ok(manifest);
            }

            if status == reqwest::StatusCode::NOT_FOUND {
                continue;
            }

            let body = response.text().await.unwrap_or_default();
            anyhow::bail!(
                "Failed to fetch plugin.toml for {} on {}: {} {}",
                repo_name,
                branch,
                status,
                body
            );
        }

        anyhow::bail!("plugin.toml not found for {} on main or master branch", repo_name)
    }

    async fn fetch_release_version(
        &self,
        repo_name: &str,
        manifest: &crate::plugins::PluginManifest,
    ) -> Result<String> {
        let target = PlatformTarget::current()?;
        let plugin_repo = format!("{}/{}", self.org, repo_name);
        let plugin_release = self.fetch_latest_release(&plugin_repo).await?;
        self
            .verify_platform_binary_support(manifest, target, &plugin_repo, &plugin_release)
            .await?;
        normalize_semver_tag(&plugin_release.tag_name).ok_or_else(|| {
            anyhow::anyhow!(
                "latest release tag '{}' is not valid semver",
                plugin_release.tag_name
            )
        })
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
            if !release_cache.contains_key(&required_asset.repo) {
                let release = self.fetch_latest_release(&required_asset.repo).await?;
                release_cache.insert(required_asset.repo.clone(), release);
            }

            let has_asset = release_cache
                .get(&required_asset.repo)
                .map(|release| {
                    release
                        .assets
                        .iter()
                        .any(|asset| asset.name == required_asset.name)
                })
                .unwrap_or(false);

            if !has_asset {
                anyhow::bail!(
                    "missing asset '{}' in latest release of {}",
                    required_asset.name,
                    required_asset.repo
                );
            }
        }

        Ok(())
    }

    async fn fetch_latest_release(&self, repo: &str) -> Result<GitHubRelease> {
        let url = format!("https://api.github.com/repos/{}/releases/latest", repo);
        let response = send_checked(self.build_request(&url)).await?;
        let release: GitHubRelease = response.json().await?;
        Ok(release)
    }

    pub async fn list_plugins_cached(&self, force_refresh: bool) -> Result<Vec<PluginMetadata>> {
        if !force_refresh {
            if let Some(plugins) = get_valid_cache() {
                return Ok(plugins);
            }
        }

        log::info!("Fetching fresh plugin data from GitHub");
        let plugins = self.list_plugins().await?;

        if let Err(e) = write_cache(&plugins) {
            log::warn!("Failed to write plugin cache: {}", e);
        }

        Ok(plugins)
    }
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
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn filter_plugin_repos(repos: &[GitHubRepo]) -> Vec<&GitHubRepo> {
    repos.iter().filter(|r| is_plugin_repo(&r.name)).collect()
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

    Ok(
        dependencies
            .binaries
            .iter()
            .map(|binary| RequiredReleaseAsset {
                repo: binary.repo.clone(),
                name: resolve_asset_pattern(&binary.pattern, target),
            })
            .collect(),
    )
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
            let repos: Vec<_> = input_names.iter().map(|n| make_repo(n)).collect();
            let filtered = filter_plugin_repos(&repos);
            let names: Vec<_> = filtered.iter().map(|r| r.name.as_str()).collect();
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
            platforms: platforms.map(|p| p.into_iter().map(String::from).collect()),
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
