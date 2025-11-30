use super::PluginInfo;
use anyhow::Result;

pub struct PluginRepository {
    org: String,
}

impl PluginRepository {
    pub fn new(org: impl Into<String>) -> Self {
        Self { org: org.into() }
    }

    pub async fn list_plugins(&self) -> Result<Vec<PluginInfo>> {
        let url = format!("https://api.github.com/orgs/{}/repos", self.org);

        let client = reqwest::Client::builder()
            .user_agent("qol-tray")
            .build()?;

        let repos: Vec<GitHubRepo> = client
            .get(&url)
            .send()
            .await?
            .json()
            .await?;

        let plugins = repos
            .into_iter()
            .filter(|r| r.name.starts_with("plugin-"))
            .map(|r| PluginInfo {
                name: r.name.strip_prefix("plugin-").unwrap_or(&r.name).to_string(),
                description: r.description.unwrap_or_default(),
                repo_url: r.clone_url,
            })
            .collect();

        Ok(plugins)
    }

    pub fn get_clone_url(&self, plugin_name: &str) -> String {
        format!("https://github.com/{}/plugin-{}.git", self.org, plugin_name)
    }
}

#[derive(serde::Deserialize)]
struct GitHubRepo {
    name: String,
    description: Option<String>,
    clone_url: String,
}
