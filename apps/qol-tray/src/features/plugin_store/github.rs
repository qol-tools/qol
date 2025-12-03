use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const PLUGIN_PREFIX: &str = "plugin-";
const CACHE_TTL_SECS: u64 = 3600;

fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("qol-tray")
}

fn token_path() -> PathBuf {
    config_dir().join(".github-token")
}

fn cache_path() -> PathBuf {
    config_dir().join(".plugin-cache.json")
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PluginCache {
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
}

impl From<PluginMetadata> for CachedPlugin {
    fn from(m: PluginMetadata) -> Self {
        Self {
            id: m.id,
            name: m.name,
            description: m.description,
            version: m.version,
            repo_url: m.repo_url,
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
    let path = cache_path();
    let content = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&content).ok()
}

pub fn write_cache(plugins: &[PluginMetadata]) -> Result<()> {
    let path = cache_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let cache = PluginCache {
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

fn get_valid_cache() -> Option<Vec<PluginMetadata>> {
    let cache = read_cache()?;
    let age = current_timestamp() - cache.timestamp;
    
    if age >= CACHE_TTL_SECS {
        return None;
    }
    
    log::info!("Using cached plugin data ({} seconds old)", age);
    Some(cache.plugins.into_iter().map(PluginMetadata::from).collect())
}

#[derive(Debug, Deserialize)]
struct GitHubRepo {
    name: String,
    #[allow(dead_code)]
    description: Option<String>,
    html_url: String,
}

#[derive(Debug, Deserialize)]
struct GitHubTag {
    name: String,
}

pub struct GitHubClient {
    org: String,
    client: reqwest::Client,
    token: Option<String>,
}

pub fn get_stored_token() -> Option<String> {
    let path = token_path();
    let token = std::fs::read_to_string(&path).ok()?;
    let token = token.trim();
    
    if token.is_empty() {
        return None;
    }
    
    log::info!("Loaded GitHub token from {:?}", path);
    Some(token.to_string())
}

pub fn store_token(token: &str) -> Result<()> {
    let path = token_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, token.trim())?;
    log::info!("Stored GitHub token to {:?}", path);
    Ok(())
}

pub fn delete_token() -> Result<()> {
    let path = token_path();
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
        let mut req = self.client
            .get(url)
            .header("User-Agent", "qol-tray");
        
        if let Some(token) = &self.token {
            req = req.header("Authorization", format!("Bearer {}", token));
        }
        
        req
    }

    pub async fn list_plugins(&self) -> Result<Vec<PluginMetadata>> {
        let url = format!("https://api.github.com/orgs/{}/repos", self.org);

        let response = self.build_request(&url)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("GitHub API returned {}: {}", status, body);
        }

        let repos: Vec<GitHubRepo> = response.json().await?;

        let plugin_repos = filter_plugin_repos(&repos);
        let mut plugins = Vec::new();

        for repo in plugin_repos {
            if let Ok(manifest) = self.fetch_plugin_manifest(&repo.name).await {
                let tag_version = self.fetch_latest_tag(&repo.name).await;
                plugins.push(build_plugin_metadata(repo, manifest, tag_version));
            }
        }

        Ok(plugins)
    }

    async fn fetch_plugin_manifest(&self, repo_name: &str) -> Result<crate::plugins::PluginManifest> {
        for branch in ["main", "master"] {
            let url = format!(
                "https://raw.githubusercontent.com/{}/{}/{}/plugin.toml",
                self.org, repo_name, branch
            );

            let response = self.build_request(&url).send().await?;
            if response.status().is_success() {
                let content = response.text().await?;
                let manifest: crate::plugins::PluginManifest = toml::from_str(&content)?;
                return Ok(manifest);
            }
        }

        anyhow::bail!("plugin.toml not found on main or master branch")
    }

    async fn fetch_latest_tag(&self, repo_name: &str) -> Option<String> {
        let url = format!(
            "https://api.github.com/repos/{}/{}/tags?per_page=1",
            self.org, repo_name
        );

        let response = self.build_request(&url).send().await.ok()?;
        if !response.status().is_success() {
            return None;
        }

        let tags: Vec<GitHubTag> = response.json().await.ok()?;
        tags.first().map(|t| t.name.trim_start_matches('v').to_string())
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
    name.starts_with(PLUGIN_PREFIX)
}

fn filter_plugin_repos(repos: &[GitHubRepo]) -> Vec<&GitHubRepo> {
    repos.iter().filter(|r| is_plugin_repo(&r.name)).collect()
}

fn build_plugin_metadata(
    repo: &GitHubRepo,
    manifest: crate::plugins::PluginManifest,
    tag_version: Option<String>,
) -> PluginMetadata {
    PluginMetadata {
        id: repo.name.clone(),
        name: manifest.plugin.name,
        description: manifest.plugin.description,
        version: tag_version.unwrap_or(manifest.plugin.version),
        repo_url: repo.html_url.clone(),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PluginMetadata {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub repo_url: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::manifest::{PluginManifest, PluginInfo, MenuConfig};

    fn make_repo(name: &str) -> GitHubRepo {
        GitHubRepo {
            name: name.to_string(),
            description: Some("Test repo".to_string()),
            html_url: format!("https://github.com/test/{}", name),
        }
    }

    fn make_manifest(name: &str, version: &str) -> PluginManifest {
        PluginManifest {
            plugin: PluginInfo {
                name: name.to_string(),
                description: "Test plugin".to_string(),
                version: version.to_string(),
                author: None,
            },
            menu: MenuConfig {
                label: "Test".to_string(),
                icon: None,
                items: vec![],
            },
            daemon: None,
            dependencies: None,
        }
    }

    #[test]
    fn is_plugin_repo_returns_true_for_plugin_prefix() {
        // Arrange
        let name = "plugin-screen-recorder";

        // Act
        let result = is_plugin_repo(name);

        // Assert
        assert!(result);
    }

    #[test]
    fn is_plugin_repo_returns_false_for_non_plugin_prefix() {
        // Arrange
        let names = ["screen-recorder", "my-plugin", "pluginstore", ""];

        // Act & Assert
        for name in names {
            assert!(!is_plugin_repo(name), "Expected false for '{}'", name);
        }
    }

    #[test]
    fn filter_plugin_repos_returns_only_plugin_prefixed_repos() {
        // Arrange
        let repos = vec![
            make_repo("plugin-recorder"),
            make_repo("some-tool"),
            make_repo("plugin-notes"),
            make_repo("pluginish"),
        ];

        // Act
        let filtered = filter_plugin_repos(&repos);

        // Assert
        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].name, "plugin-recorder");
        assert_eq!(filtered[1].name, "plugin-notes");
    }

    #[test]
    fn filter_plugin_repos_returns_empty_when_no_plugins() {
        // Arrange
        let repos = vec![
            make_repo("tool-one"),
            make_repo("tool-two"),
        ];

        // Act
        let filtered = filter_plugin_repos(&repos);

        // Assert
        assert!(filtered.is_empty());
    }

    #[test]
    fn build_plugin_metadata_uses_tag_version_when_available() {
        // Arrange
        let repo = make_repo("plugin-screen-recorder");
        let manifest = make_manifest("Screen Recorder", "1.0.0");

        // Act
        let metadata = build_plugin_metadata(&repo, manifest, Some("2.0.0".to_string()));

        // Assert
        assert_eq!(metadata.id, "plugin-screen-recorder");
        assert_eq!(metadata.name, "Screen Recorder");
        assert_eq!(metadata.version, "2.0.0");
    }

    #[test]
    fn build_plugin_metadata_falls_back_to_manifest_version() {
        // Arrange
        let repo = make_repo("plugin-screen-recorder");
        let manifest = make_manifest("Screen Recorder", "1.2.3");

        // Act
        let metadata = build_plugin_metadata(&repo, manifest, None);

        // Assert
        assert_eq!(metadata.version, "1.2.3");
    }
}
