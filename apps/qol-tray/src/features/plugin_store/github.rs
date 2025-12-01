use anyhow::Result;
use serde::Deserialize;
use std::path::PathBuf;

const PLUGIN_PREFIX: &str = "plugin-";

fn token_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("qol-tray")
        .join(".github-token")
}

#[derive(Debug, Deserialize)]
struct GitHubRepo {
    name: String,
    #[allow(dead_code)]
    description: Option<String>,
    html_url: String,
}

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
}

pub struct GitHubClient {
    org: String,
    client: reqwest::Client,
    token: Option<String>,
}

pub fn get_stored_token() -> Option<String> {
    let path = token_path();
    match std::fs::read_to_string(&path) {
        Ok(token) => {
            let token = token.trim().to_string();
            if token.is_empty() {
                None
            } else {
                log::info!("Loaded GitHub token from {:?}", path);
                Some(token)
            }
        }
        Err(_) => None,
    }
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
                let version = self.resolve_version(&manifest).await;
                plugins.push(build_plugin_metadata(repo, manifest, version));
            }
        }

        Ok(plugins)
    }

    async fn fetch_plugin_manifest(&self, repo_name: &str) -> Result<crate::plugins::PluginManifest> {
        let url = format!(
            "https://raw.githubusercontent.com/{}/{}/main/plugin.toml",
            self.org, repo_name
        );

        let content = self.build_request(&url)
            .send()
            .await?
            .text()
            .await?;

        let manifest: crate::plugins::PluginManifest = toml::from_str(&content)?;
        Ok(manifest)
    }

    async fn resolve_version(&self, manifest: &crate::plugins::PluginManifest) -> String {
        if let Some(deps) = &manifest.dependencies {
            if let Some(binary) = deps.binaries.first() {
                if let Ok(version) = self.fetch_latest_release(&binary.repo).await {
                    return version;
                }
            }
        }
        manifest.plugin.version.clone()
    }

    async fn fetch_latest_release(&self, repo: &str) -> Result<String> {
        let url = format!("https://api.github.com/repos/{}/releases/latest", repo);

        let response = self.build_request(&url)
            .send()
            .await?;

        if !response.status().is_success() {
            anyhow::bail!("GitHub API returned {}", response.status());
        }

        let release: GitHubRelease = response.json().await?;
        Ok(release.tag_name.trim_start_matches('v').to_string())
    }

    pub fn has_token(&self) -> bool {
        self.token.is_some()
    }
}

fn is_plugin_repo(name: &str) -> bool {
    name.starts_with(PLUGIN_PREFIX)
}

fn filter_plugin_repos(repos: &[GitHubRepo]) -> Vec<&GitHubRepo> {
    repos.iter().filter(|r| is_plugin_repo(&r.name)).collect()
}

fn build_plugin_metadata(repo: &GitHubRepo, manifest: crate::plugins::PluginManifest, version: String) -> PluginMetadata {
    PluginMetadata {
        id: repo.name.clone(),
        name: manifest.plugin.name,
        description: manifest.plugin.description,
        version,
        repo_url: repo.html_url.clone(),
    }
}

#[derive(Debug, PartialEq)]
pub struct PluginMetadata {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    #[allow(dead_code)]
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
    fn build_plugin_metadata_extracts_fields_from_repo_and_manifest() {
        // Arrange
        let repo = make_repo("plugin-screen-recorder");
        let manifest = make_manifest("Screen Recorder", "1.2.3");
        let version = "2.0.0".to_string();

        // Act
        let metadata = build_plugin_metadata(&repo, manifest, version);

        // Assert
        assert_eq!(metadata.id, "plugin-screen-recorder");
        assert_eq!(metadata.name, "Screen Recorder");
        assert_eq!(metadata.description, "Test plugin");
        assert_eq!(metadata.version, "2.0.0");
        assert_eq!(metadata.repo_url, "https://github.com/test/plugin-screen-recorder");
    }
}
