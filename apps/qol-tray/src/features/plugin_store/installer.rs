use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const GIT_TIMEOUT: Duration = Duration::from_secs(120);

pub struct PluginInstaller {
    plugins_dir: PathBuf,
}

impl PluginInstaller {
    pub fn new(plugins_dir: PathBuf) -> Self {
        Self { plugins_dir }
    }

    pub async fn install(&self, repo_url: &str, plugin_id: &str) -> Result<()> {
        Self::check_dev_link_conflict(plugin_id)?;

        let target_dir = self.plugins_dir.join(plugin_id);

        if target_dir.exists() {
            anyhow::bail!("Plugin already installed: {}", plugin_id);
        }

        let staging_dir = self.install_staging_dir(plugin_id);
        log::info!("Cloning plugin from {} to {:?}", repo_url, staging_dir);

        let staging_str = staging_dir
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("Plugin path contains invalid UTF-8"))?;

        let output = tokio::time::timeout(
            GIT_TIMEOUT,
            tokio::process::Command::new("git")
                .args(["clone", repo_url, staging_str])
                .output(),
        )
        .await
        .context("Git clone timed out")??;

        if !output.status.success() {
            self.cleanup_temp_dir(&staging_dir).await;
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Git clone failed: {}", stderr);
        }

        if let Err(error) = self.install_dependencies(&staging_dir, plugin_id).await {
            self.cleanup_temp_dir(&staging_dir).await;
            return Err(error);
        }

        if let Err(error) = tokio::fs::rename(&staging_dir, &target_dir).await {
            self.cleanup_temp_dir(&staging_dir).await;
            return Err(error).with_context(|| {
                format!(
                    "Failed to finalize plugin install from {:?} to {:?}",
                    staging_dir, target_dir
                )
            });
        }

        log::info!("Plugin {} installed successfully", plugin_id);
        Ok(())
    }

    fn install_staging_dir(&self, plugin_id: &str) -> PathBuf {
        self.temporary_plugin_dir(plugin_id, "installing")
    }

    fn update_staging_dir(&self, plugin_id: &str) -> PathBuf {
        self.temporary_plugin_dir(plugin_id, "updating")
    }

    fn update_backup_dir(&self, plugin_id: &str) -> PathBuf {
        self.temporary_plugin_dir(plugin_id, "backup")
    }

    fn temporary_plugin_dir(&self, plugin_id: &str, phase: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        self.plugins_dir.join(format!(
            ".{}.{}.{}.{}",
            plugin_id,
            phase,
            std::process::id(),
            nanos
        ))
    }

    async fn cleanup_temp_dir(&self, path: &Path) {
        if tokio::fs::metadata(path).await.is_err() {
            return;
        }

        if let Err(error) = tokio::fs::remove_dir_all(path).await {
            log::warn!(
                "Failed to cleanup temporary plugin directory {:?}: {}",
                path,
                error
            );
        }
    }

    async fn swap_plugin_dirs(
        &self,
        live_dir: &Path,
        staging_dir: &Path,
        backup_dir: &Path,
    ) -> Result<()> {
        tokio::fs::rename(live_dir, backup_dir).await.with_context(|| {
            format!(
                "Failed to move plugin directory {:?} to backup {:?}",
                live_dir, backup_dir
            )
        })?;

        match tokio::fs::rename(staging_dir, live_dir).await {
            Ok(()) => {
                if let Err(error) = tokio::fs::remove_dir_all(backup_dir).await {
                    log::warn!(
                        "Failed to cleanup plugin backup directory {:?}: {}",
                        backup_dir,
                        error
                    );
                }
                Ok(())
            }
            Err(swap_error) => {
                if let Err(rollback_error) = tokio::fs::rename(backup_dir, live_dir).await {
                    anyhow::bail!(
                        "Failed to swap plugin directories: {}; rollback failed: {}",
                        swap_error,
                        rollback_error
                    );
                }
                anyhow::bail!("Failed to swap plugin directories: {}", swap_error);
            }
        }
    }

    async fn install_dependencies(&self, plugin_dir: &Path, plugin_id: &str) -> Result<()> {
        let manifest_path = plugin_dir.join("plugin.toml");
        if !manifest_path.exists() {
            anyhow::bail!("Missing plugin.toml in {:?}", plugin_dir);
        }

        let content = tokio::fs::read_to_string(&manifest_path)
            .await
            .with_context(|| format!("Failed to read {:?}", manifest_path))?;
        let manifest: crate::plugins::PluginManifest = toml::from_str(&content)
            .context("Failed to parse plugin.toml")?;
        manifest
            .validate()
            .context("Invalid plugin.toml contract")?;

        if let Some(deps) = manifest.dependencies.as_ref() {
            for binary in &deps.binaries {
                self.install_binary(plugin_dir, binary).await?;
            }
        }

        if manifest.plugin.supports_current_platform() {
            crate::plugins::validate_execution_contract(plugin_id, &manifest, plugin_dir)
                .context("Plugin binary contract preflight failed")?;
        }

        Ok(())
    }

    async fn install_binary(
        &self,
        plugin_dir: &Path,
        dep: &crate::plugins::manifest::BinaryDependency,
    ) -> Result<()> {
        let asset_name = resolve_asset_pattern(&dep.pattern);
        log::info!("Fetching {} from {}", asset_name, dep.repo);

        let release = fetch_latest_release(&dep.repo).await?;
        let asset = release
            .assets
            .iter()
            .find(|a| a.name == asset_name)
            .with_context(|| format!("Asset '{}' not found in release", asset_name))?;

        let binary_path = plugin_dir.join(&dep.name);
        download_asset(&asset.browser_download_url, &binary_path).await?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = tokio::fs::metadata(&binary_path).await?.permissions();
            perms.set_mode(0o755);
            tokio::fs::set_permissions(&binary_path, perms).await?;
        }

        log::info!("Installed binary: {:?}", binary_path);
        Ok(())
    }

    pub async fn update(&self, plugin_id: &str) -> Result<()> {
        Self::check_dev_link_conflict(plugin_id)?;

        let plugin_dir = self.plugins_dir.join(plugin_id);

        if !plugin_dir.exists() {
            anyhow::bail!("Plugin not installed: {}", plugin_id);
        }

        let staging_dir = self.update_staging_dir(plugin_id);
        let backup_dir = self.update_backup_dir(plugin_id);

        log::info!("Updating plugin: {}", plugin_id);

        let plugin_dir_str = plugin_dir
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("Plugin path contains invalid UTF-8"))?;
        let staging_str = staging_dir
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("Plugin path contains invalid UTF-8"))?;

        let clone_output = tokio::time::timeout(
            GIT_TIMEOUT,
            tokio::process::Command::new("git")
                .args(["clone", plugin_dir_str, staging_str])
                .output(),
        )
        .await
        .context("Git clone timed out")??;

        if !clone_output.status.success() {
            self.cleanup_temp_dir(&staging_dir).await;
            let stderr = String::from_utf8_lossy(&clone_output.stderr);
            anyhow::bail!("Git clone failed: {}", stderr);
        }

        let output = tokio::time::timeout(
            GIT_TIMEOUT,
            tokio::process::Command::new("git")
                .args(["fetch", "origin"])
                .current_dir(&staging_dir)
                .output(),
        )
        .await
        .context("Git fetch timed out")??;

        if !output.status.success() {
            self.cleanup_temp_dir(&staging_dir).await;
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Git fetch failed: {}", stderr);
        }

        let branch = match self.get_default_branch(&staging_dir).await {
            Ok(branch) => branch,
            Err(error) => {
                self.cleanup_temp_dir(&staging_dir).await;
                return Err(error);
            }
        };
        let output = tokio::time::timeout(
            GIT_TIMEOUT,
            tokio::process::Command::new("git")
                .args(["reset", "--hard", &format!("origin/{}", branch)])
                .current_dir(&staging_dir)
                .output(),
        )
        .await
        .context("Git reset timed out")??;

        if !output.status.success() {
            self.cleanup_temp_dir(&staging_dir).await;
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Git reset failed: {}", stderr);
        }

        if let Err(error) = self.install_dependencies(&staging_dir, plugin_id).await {
            self.cleanup_temp_dir(&staging_dir).await;
            return Err(error);
        }

        if let Err(error) = self
            .swap_plugin_dirs(&plugin_dir, &staging_dir, &backup_dir)
            .await
        {
            self.cleanup_temp_dir(&staging_dir).await;
            return Err(error);
        }

        log::info!("Plugin {} updated successfully", plugin_id);
        Ok(())
    }

    async fn get_default_branch(&self, plugin_dir: &Path) -> Result<String> {
        let output = tokio::time::timeout(
            Duration::from_secs(10),
            tokio::process::Command::new("git")
                .args(["symbolic-ref", "refs/remotes/origin/HEAD", "--short"])
                .current_dir(plugin_dir)
                .output(),
        )
        .await
        .context("Git symbolic-ref timed out")??;

        if output.status.success() {
            let branch = String::from_utf8_lossy(&output.stdout);
            let branch = branch.trim().trim_start_matches("origin/");
            if is_safe_branch_name(branch) {
                return Ok(branch.to_string());
            }
            log::warn!("Invalid branch name from git: {:?}", branch);
        }

        Ok("master".to_string())
    }

    #[cfg(feature = "dev")]
    fn check_dev_link_conflict(plugin_id: &str) -> Result<()> {
        let config_dir = crate::paths::config_dir()?;
        let dev_links = crate::dev::load_dev_links(&config_dir);
        if dev_links.contains_key(plugin_id) {
            anyhow::bail!(
                "Cannot proceed — {} is dev-linked. Unlink first.",
                plugin_id
            );
        }
        Ok(())
    }

    #[cfg(not(feature = "dev"))]
    fn check_dev_link_conflict(_plugin_id: &str) -> Result<()> {
        Ok(())
    }

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

fn is_safe_branch_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 256
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '/' || c == '.')
        && !s.starts_with('-')
        && !s.starts_with('.')
        && !s.contains("..")
}

fn resolve_asset_pattern(pattern: &str) -> String {
    let os = get_os_name();
    let arch = get_arch_name();
    let ext = if cfg!(windows) { ".exe" } else { "" };

    pattern.replace("{os}", os).replace("{arch}", arch) + ext
}

fn get_os_name() -> &'static str {
    if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "unknown"
    }
}

fn get_arch_name() -> &'static str {
    if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        "unknown"
    }
}

#[derive(Deserialize)]
struct GitHubRelease {
    assets: Vec<GitHubAsset>,
}

#[derive(Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
}

async fn fetch_latest_release(repo: &str) -> Result<GitHubRelease> {
    let url = format!("https://api.github.com/repos/{}/releases/latest", repo);
    let client = reqwest::Client::new();

    let release: GitHubRelease = client
        .get(&url)
        .header("User-Agent", "qol-tray")
        .send()
        .await?
        .json()
        .await?;

    Ok(release)
}

async fn download_asset(url: &str, dest: &PathBuf) -> Result<()> {
    let client = reqwest::Client::new();
    let response = client
        .get(url)
        .header("User-Agent", "qol-tray")
        .send()
        .await?;

    let bytes = response.bytes().await?;
    tokio::fs::write(dest, &bytes).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn is_safe_branch_name_cases() {
        let valid = [
            "main",
            "master",
            "develop",
            "feature/foo",
            "release-1.0",
            "v1.0.0",
            "fix_bug",
            "a",
        ];
        for s in valid {
            assert!(is_safe_branch_name(s), "should be valid: {:?}", s);
        }

        let invalid = [
            "",
            "-leading-dash",
            ".hidden",
            "has..double-dots",
            "has\nline",
            "has\ttab",
            "has space",
            &"x".repeat(300),
        ];
        for s in invalid {
            assert!(!is_safe_branch_name(s), "should be invalid: {:?}", s);
        }
    }

    #[test]
    fn install_staging_dir_uses_hidden_plugin_scoped_prefix() {
        let root = TempDir::new().unwrap();
        let installer = PluginInstaller::new(root.path().to_path_buf());

        let staging = installer.install_staging_dir("plugin-test");
        let name = staging.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with(".plugin-test.installing."));
    }

    #[test]
    fn update_staging_dir_uses_hidden_plugin_scoped_prefix() {
        let root = TempDir::new().unwrap();
        let installer = PluginInstaller::new(root.path().to_path_buf());

        let staging = installer.update_staging_dir("plugin-test");
        let name = staging.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with(".plugin-test.updating."));
    }

    #[test]
    fn update_backup_dir_uses_hidden_plugin_scoped_prefix() {
        let root = TempDir::new().unwrap();
        let installer = PluginInstaller::new(root.path().to_path_buf());

        let backup = installer.update_backup_dir("plugin-test");
        let name = backup.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with(".plugin-test.backup."));
    }

    #[tokio::test]
    async fn cleanup_temp_dir_removes_directory_tree() {
        let root = TempDir::new().unwrap();
        let installer = PluginInstaller::new(root.path().to_path_buf());
        let install_dir = root.path().join(".plugin-test.installing.1");
        tokio::fs::create_dir_all(install_dir.join("nested")).await.unwrap();
        tokio::fs::write(install_dir.join("nested").join("file.txt"), b"x")
            .await
            .unwrap();

        installer.cleanup_temp_dir(&install_dir).await;
        assert!(!install_dir.exists());
    }

    #[tokio::test]
    async fn swap_plugin_dirs_replaces_live_with_staging() {
        let root = TempDir::new().unwrap();
        let installer = PluginInstaller::new(root.path().to_path_buf());
        let live = root.path().join("plugin-test");
        let staging = root.path().join(".plugin-test.updating.1");
        let backup = root.path().join(".plugin-test.backup.1");

        tokio::fs::create_dir_all(&live).await.unwrap();
        tokio::fs::create_dir_all(&staging).await.unwrap();
        tokio::fs::write(live.join("old.txt"), b"old").await.unwrap();
        tokio::fs::write(staging.join("new.txt"), b"new").await.unwrap();

        installer
            .swap_plugin_dirs(&live, &staging, &backup)
            .await
            .unwrap();

        assert!(tokio::fs::metadata(live.join("new.txt")).await.is_ok());
        assert!(tokio::fs::metadata(live.join("old.txt")).await.is_err());
        assert!(tokio::fs::metadata(&backup).await.is_err());
    }

    #[tokio::test]
    async fn swap_plugin_dirs_rolls_back_when_staging_missing() {
        let root = TempDir::new().unwrap();
        let installer = PluginInstaller::new(root.path().to_path_buf());
        let live = root.path().join("plugin-test");
        let staging = root.path().join(".plugin-test.updating.1");
        let backup = root.path().join(".plugin-test.backup.1");

        tokio::fs::create_dir_all(&live).await.unwrap();
        tokio::fs::write(live.join("old.txt"), b"old").await.unwrap();

        assert!(
            installer
                .swap_plugin_dirs(&live, &staging, &backup)
                .await
                .is_err()
        );
        assert!(tokio::fs::metadata(live.join("old.txt")).await.is_ok());
        assert!(tokio::fs::metadata(&backup).await.is_err());
    }
}
