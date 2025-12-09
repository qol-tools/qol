use crate::paths;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const PLUGIN_PREFIX: &str = "plugin-";
const CACHE_TTL_SECS: u64 = 3600;

fn token_path() -> PathBuf {
    paths::github_token_path().expect("failed to determine config directory")
}

fn cache_path() -> PathBuf {
    paths::plugin_cache_path().expect("failed to determine config directory")
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

pub fn update_cached_version(plugin_id: &str, version: &str) {
    let Some(mut cache) = read_cache() else { return };
    let Some(plugin) = cache.plugins.iter_mut().find(|p| p.id == plugin_id) else { return };

    plugin.version = version.to_string();
    let Ok(content) = serde_json::to_string(&cache) else { return };
    let _ = std::fs::write(cache_path(), content);
    log::info!("Updated cache version for {}: {}", plugin_id, version);
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
    html_url: String,
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
                plugins.push(build_plugin_metadata(repo, manifest));
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
    name.starts_with(PLUGIN_PREFIX) && name != "plugin-template"
}

fn filter_plugin_repos(repos: &[GitHubRepo]) -> Vec<&GitHubRepo> {
    repos.iter().filter(|r| is_plugin_repo(&r.name)).collect()
}

fn build_plugin_metadata(repo: &GitHubRepo, manifest: crate::plugins::PluginManifest) -> PluginMetadata {
    PluginMetadata {
        id: repo.name.clone(),
        name: manifest.plugin.name,
        description: manifest.plugin.description,
        version: manifest.plugin.version,
        repo_url: repo.html_url.clone(),
        platforms: manifest.plugin.platforms,
    }
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
        match &self.platforms {
            None => true,
            Some(platforms) => platforms.iter().any(|p| p == std::env::consts::OS),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::manifest::{PluginManifest, PluginInfo, MenuConfig};

    fn make_repo(name: &str) -> GitHubRepo {
        GitHubRepo {
            name: name.to_string(),
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
                platforms: None,
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
    fn is_plugin_repo_filtering() {
        let cases = [
            ("plugin-screen-recorder", true),
            ("plugin-notes", true),
            ("plugin-a", true),
            ("plugin-123", true),
            ("plugin-CAPS", true),
            ("screen-recorder", false),
            ("my-plugin", false),
            ("pluginstore", false),
            ("plugin-template", false),
            ("", false),
            ("plugin-", true),
            ("plugin", false),
            ("PLUGIN-foo", false),
            ("Plugin-foo", false),
            (" plugin-foo", false),
            ("plugin-foo ", true),
            ("plugin--double", true),
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
            (
                vec!["tool-one", "tool-two"],
                vec![],
            ),
        ];

        for (input_names, expected_names) in cases {
            let repos: Vec<_> = input_names.iter().map(|n| make_repo(n)).collect();
            let filtered = filter_plugin_repos(&repos);
            let names: Vec<_> = filtered.iter().map(|r| r.name.as_str()).collect();
            assert_eq!(names, expected_names, "input: {:?}", input_names);
        }
    }

    #[test]
    fn build_plugin_metadata_uses_manifest_version() {
        let repo = make_repo("plugin-test");
        let manifest = make_manifest("Test", "1.0.0");
        let metadata = build_plugin_metadata(&repo, manifest);
        assert_eq!(metadata.version, "1.0.0");
    }
}
