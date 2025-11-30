use super::github::PluginRepository;
use crate::features::plugin_manager::PluginLoader;
use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process::Command;

pub struct PluginInstaller {
    plugins_dir: PathBuf,
}

impl PluginInstaller {
    pub fn new() -> Result<Self> {
        let plugins_dir = PluginLoader::ensure_plugin_dir()?;
        Ok(Self { plugins_dir })
    }

    pub async fn install(&self, repository: &PluginRepository, plugin_name: &str) -> Result<()> {
        let plugin_path = self.plugins_dir.join(plugin_name);

        if plugin_path.exists() {
            anyhow::bail!("Plugin {} is already installed", plugin_name);
        }

        let clone_url = repository.get_clone_url(plugin_name);
        log::info!("Installing plugin {} from {}", plugin_name, clone_url);

        let output = Command::new("git")
            .args(["clone", &clone_url, plugin_name])
            .current_dir(&self.plugins_dir)
            .output()
            .context("Failed to clone plugin repository")?;

        if !output.status.success() {
            anyhow::bail!(
                "Failed to clone plugin: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        self.make_executable(&plugin_path)?;

        log::info!("Plugin {} installed successfully", plugin_name);
        Ok(())
    }

    pub async fn update(&self, plugin_name: &str) -> Result<()> {
        let plugin_path = self.plugins_dir.join(plugin_name);

        if !plugin_path.exists() {
            anyhow::bail!("Plugin {} is not installed", plugin_name);
        }

        log::info!("Updating plugin {}", plugin_name);

        let output = Command::new("git")
            .args(["pull"])
            .current_dir(&plugin_path)
            .output()
            .context("Failed to update plugin")?;

        if !output.status.success() {
            anyhow::bail!(
                "Failed to update plugin: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        self.make_executable(&plugin_path)?;

        log::info!("Plugin {} updated successfully", plugin_name);
        Ok(())
    }

    pub fn uninstall(&self, plugin_name: &str) -> Result<()> {
        let plugin_path = self.plugins_dir.join(plugin_name);

        if !plugin_path.exists() {
            anyhow::bail!("Plugin {} is not installed", plugin_name);
        }

        log::info!("Uninstalling plugin {}", plugin_name);
        std::fs::remove_dir_all(&plugin_path)
            .context("Failed to remove plugin directory")?;

        log::info!("Plugin {} uninstalled successfully", plugin_name);
        Ok(())
    }

    fn make_executable(&self, plugin_path: &PathBuf) -> Result<()> {
        let run_script = plugin_path.join("run.sh");
        if run_script.exists() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = std::fs::metadata(&run_script)?.permissions();
                perms.set_mode(0o755);
                std::fs::set_permissions(&run_script, perms)?;
            }
        }
        Ok(())
    }
}
