use anyhow::Result;
use std::path::PathBuf;

pub struct PluginInstaller {
    plugins_dir: PathBuf,
}

impl PluginInstaller {
    pub fn new(plugins_dir: PathBuf) -> Self {
        Self { plugins_dir }
    }

    pub async fn install(&self, repo_url: &str, plugin_id: &str) -> Result<()> {
        let target_dir = self.plugins_dir.join(plugin_id);

        if target_dir.exists() {
            anyhow::bail!("Plugin already installed: {}", plugin_id);
        }

        log::info!("Cloning plugin from {} to {:?}", repo_url, target_dir);

        let output = tokio::process::Command::new("git")
            .args(&["clone", repo_url, target_dir.to_str().unwrap()])
            .output()
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Git clone failed: {}", stderr);
        }

        log::info!("Plugin {} installed successfully", plugin_id);
        Ok(())
    }

    #[allow(dead_code)]
    pub async fn update(&self, plugin_id: &str) -> Result<()> {
        let plugin_dir = self.plugins_dir.join(plugin_id);

        if !plugin_dir.exists() {
            anyhow::bail!("Plugin not installed: {}", plugin_id);
        }

        log::info!("Updating plugin: {}", plugin_id);

        let output = tokio::process::Command::new("git")
            .args(&["pull"])
            .current_dir(&plugin_dir)
            .output()
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Git pull failed: {}", stderr);
        }

        log::info!("Plugin {} updated successfully", plugin_id);
        Ok(())
    }

    #[allow(dead_code)]
    pub async fn uninstall(&self, plugin_id: &str) -> Result<()> {
        let plugin_dir = self.plugins_dir.join(plugin_id);

        if !plugin_dir.exists() {
            anyhow::bail!("Plugin not installed: {}", plugin_id);
        }

        log::info!("Uninstalling plugin: {}", plugin_id);
        tokio::fs::remove_dir_all(&plugin_dir).await?;
        log::info!("Plugin {} uninstalled successfully", plugin_id);
        Ok(())
    }
}
