use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::time::Duration;

const GIT_TIMEOUT: Duration = Duration::from_secs(120);

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

        let target_str = target_dir.to_str()
            .ok_or_else(|| anyhow::anyhow!("Plugin path contains invalid UTF-8"))?;

        let output = tokio::time::timeout(
            GIT_TIMEOUT,
            tokio::process::Command::new("git")
                .args(["clone", repo_url, target_str])
                .output(),
        )
        .await
        .context("Git clone timed out")??;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Git clone failed: {}", stderr);
        }

        self.install_dependencies(&target_dir).await?;

        log::info!("Plugin {} installed successfully", plugin_id);
        Ok(())
    }

    async fn install_dependencies(&self, plugin_dir: &Path) -> Result<()> {
        let manifest_path = plugin_dir.join("plugin.toml");
        if !manifest_path.exists() {
            return Ok(());
        }

        let content = tokio::fs::read_to_string(&manifest_path).await?;
        let manifest: crate::plugins::PluginManifest = toml::from_str(&content)?;

        let Some(deps) = manifest.dependencies else {
            return Ok(());
        };

        for binary in deps.binaries {
            self.install_binary(plugin_dir, &binary).await?;
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
        let plugin_dir = self.plugins_dir.join(plugin_id);

        if !plugin_dir.exists() {
            anyhow::bail!("Plugin not installed: {}", plugin_id);
        }

        log::info!("Updating plugin: {}", plugin_id);

        let output = tokio::time::timeout(
            GIT_TIMEOUT,
            tokio::process::Command::new("git")
                .args(["fetch", "origin"])
                .current_dir(&plugin_dir)
                .output(),
        )
        .await
        .context("Git fetch timed out")??;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Git fetch failed: {}", stderr);
        }

        let branch = self.get_default_branch(&plugin_dir).await?;
        let output = tokio::time::timeout(
            GIT_TIMEOUT,
            tokio::process::Command::new("git")
                .args(["reset", "--hard", &format!("origin/{}", branch)])
                .current_dir(&plugin_dir)
                .output(),
        )
        .await
        .context("Git reset timed out")??;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Git reset failed: {}", stderr);
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

    pattern
        .replace("{os}", os)
        .replace("{arch}", arch)
        + ext
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
}
