mod github;
mod installer;

pub use github::PluginRepository;
pub use installer::PluginInstaller;

use anyhow::Result;

pub struct PluginStore {
    repository: PluginRepository,
    installer: PluginInstaller,
}

impl PluginStore {
    pub fn new() -> Result<Self> {
        Ok(Self {
            repository: PluginRepository::new("qol-tools"),
            installer: PluginInstaller::new()?,
        })
    }

    pub async fn list_available(&self) -> Result<Vec<PluginInfo>> {
        self.repository.list_plugins().await
    }

    pub async fn install(&self, plugin_name: &str) -> Result<()> {
        self.installer.install(&self.repository, plugin_name).await
    }

    pub async fn update(&self, plugin_name: &str) -> Result<()> {
        self.installer.update(plugin_name).await
    }

    pub fn uninstall(&self, plugin_name: &str) -> Result<()> {
        self.installer.uninstall(plugin_name)
    }
}

#[derive(Debug, Clone)]
pub struct PluginInfo {
    pub name: String,
    pub description: String,
    pub repo_url: String,
}
