use super::release_assets::{resolve_asset_pattern, PlatformTarget};
use anyhow::{Context, Result};
use serde::Deserialize;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

mod command;
use command::{run_cargo_build, run_git, run_git_checked};

const GIT_TIMEOUT: Duration = Duration::from_secs(120);
const CARGO_BUILD_TIMEOUT: Duration = Duration::from_secs(300);
const DEFAULT_CARGO_BUILD_JOBS: usize = 4;
#[cfg(unix)]
const LOCKFILE_MAX_AGE: Duration = Duration::from_secs(30);
#[cfg(not(unix))]
const LOCKFILE_MAX_AGE: Duration = Duration::from_secs(300);

pub struct PluginInstaller {
    plugins_dir: PathBuf,
}

impl PluginInstaller {
    pub fn new(plugins_dir: PathBuf) -> Self {
        Self { plugins_dir }
    }

    pub async fn install(&self, repo_url: &str, plugin_id: &str) -> Result<()> {
        validate_plugin_id(plugin_id)?;
        let _operation_lock = self.acquire_operation_lock(plugin_id)?;
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

        if let Err(error) = run_git_checked(["clone", repo_url, staging_str], None, "clone").await {
            self.cleanup_temp_dir(&staging_dir).await;
            return Err(error);
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
        tokio::fs::rename(live_dir, backup_dir)
            .await
            .with_context(|| {
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
        let manifest: crate::plugins::PluginManifest =
            toml::from_str(&content).context("Failed to parse plugin.toml")?;
        manifest
            .validate()
            .context("Invalid plugin.toml contract")?;

        if let Some(deps) = manifest.dependencies.as_ref() {
            for binary in &deps.binaries {
                self.install_binary(plugin_id, plugin_dir, binary).await?;
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
        plugin_id: &str,
        plugin_dir: &Path,
        dep: &crate::plugins::manifest::BinaryDependency,
    ) -> Result<()> {
        if !crate::plugins::manifest::is_valid_command_basename(&dep.name) {
            anyhow::bail!(
                "Invalid dependency binary name {:?}; expected basename [A-Za-z0-9_-]",
                dep.name
            );
        }
        let asset_name = resolve_asset_pattern(&dep.pattern, PlatformTarget::current()?);
        log::info!("Fetching {} from {}", asset_name, dep.repo);
        let binary_path = dependency_binary_output_path(plugin_dir, &dep.name);

        let mut downloaded = false;
        match fetch_latest_release(&dep.repo).await {
            Ok(release) => {
                if let Some(asset) = release.assets.iter().find(|a| a.name == asset_name) {
                    download_asset(&asset.browser_download_url, &binary_path).await?;
                    downloaded = true;
                } else {
                    log::warn!("Release asset '{}' missing for {}", asset_name, dep.repo);
                }
            }
            Err(error) => {
                log::warn!(
                    "Failed to fetch release asset {} from {}: {:#}",
                    asset_name,
                    dep.repo,
                    error
                );
            }
        }

        if !downloaded {
            if !can_build_from_source_fallback(plugin_id, &dep.repo, plugin_dir) {
                anyhow::bail!(
                    "Asset '{}' not available for {} and source-build fallback is unavailable",
                    asset_name,
                    dep.repo
                );
            }
            log::warn!(
                "Falling back to local source build for dependency {} in {:?}",
                dep.name,
                plugin_dir
            );
            build_binary_from_source(plugin_dir, &dep.name).await?;
        }

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

    pub async fn update(&self, repo_url: &str, plugin_id: &str) -> Result<()> {
        validate_plugin_id(plugin_id)?;
        let _operation_lock = self.acquire_operation_lock(plugin_id)?;
        Self::check_dev_link_conflict(plugin_id)?;

        let plugin_dir = self.plugins_dir.join(plugin_id);

        if !plugin_dir.exists() {
            anyhow::bail!("Plugin not installed: {}", plugin_id);
        }

        let staging_dir = self.update_staging_dir(plugin_id);
        let backup_dir = self.update_backup_dir(plugin_id);

        log::info!("Updating plugin {} from {}", plugin_id, repo_url);

        let staging_str = staging_dir
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("Plugin path contains invalid UTF-8"))?;

        if let Err(error) = run_git_checked(["clone", repo_url, staging_str], None, "clone").await {
            self.cleanup_temp_dir(&staging_dir).await;
            return Err(error);
        }

        let branch = match self.get_default_branch(&staging_dir).await {
            Ok(branch) => branch,
            Err(error) => {
                self.cleanup_temp_dir(&staging_dir).await;
                return Err(error);
            }
        };
        if let Err(error) = self.reset_to_origin_head(&staging_dir, &branch).await {
            self.cleanup_temp_dir(&staging_dir).await;
            return Err(error);
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
        let output = run_git(
            ["symbolic-ref", "refs/remotes/origin/HEAD", "--short"],
            Some(plugin_dir),
            Duration::from_secs(10),
            "symbolic-ref",
        )
        .await?;

        if output.status.success() {
            let branch = String::from_utf8_lossy(&output.stdout);
            let branch = branch.trim().trim_start_matches("origin/");
            if is_safe_branch_name(branch) {
                return Ok(branch.to_string());
            }
            log::warn!("Invalid branch name from git: {:?}", branch);
        }

        Ok("main".to_string())
    }

    async fn reset_to_origin_head(&self, plugin_dir: &Path, branch: &str) -> Result<()> {
        let mut candidates = vec![branch.to_string()];
        if branch != "main" {
            candidates.push("main".to_string());
        }
        if branch != "master" {
            candidates.push("master".to_string());
        }
        let mut last_error = String::new();

        for candidate in candidates {
            let reset_target = format!("origin/{}", candidate);
            let output = run_git(
                ["reset", "--hard", reset_target.as_str()],
                Some(plugin_dir),
                GIT_TIMEOUT,
                "reset",
            )
            .await?;

            if output.status.success() {
                return Ok(());
            }
            last_error = String::from_utf8_lossy(&output.stderr).trim().to_string();
        }

        if last_error.is_empty() {
            anyhow::bail!("Git reset failed for origin/main and origin/master");
        }
        anyhow::bail!(
            "Git reset failed for origin/main and origin/master: {}",
            last_error
        )
    }

    #[cfg(feature = "dev")]
    fn check_dev_link_conflict(plugin_id: &str) -> Result<()> {
        let config_dir = crate::paths::shared_config_dir()?;
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
        validate_plugin_id(plugin_id)?;
        let _operation_lock = self.acquire_operation_lock(plugin_id)?;
        let plugin_dir = self.plugins_dir.join(plugin_id);

        if !plugin_dir.exists() {
            anyhow::bail!("Plugin not installed: {}", plugin_id);
        }

        log::info!("Uninstalling plugin: {}", plugin_id);
        tokio::fs::remove_dir_all(&plugin_dir).await?;
        log::info!("Plugin {} uninstalled successfully", plugin_id);
        Ok(())
    }

    fn acquire_operation_lock(&self, plugin_id: &str) -> Result<PluginOperationLock> {
        std::fs::create_dir_all(&self.plugins_dir).with_context(|| {
            format!(
                "Failed to create plugins directory {}",
                self.plugins_dir.display()
            )
        })?;
        let path = self.plugins_dir.join(format!(".{}.lock", plugin_id));

        match open_lock_file(&path) {
            Ok(mut file) => {
                let _ = writeln!(file, "{} {}", std::process::id(), plugin_id);
                Ok(PluginOperationLock { path })
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                if stale_lockfile(&path) {
                    let _ = std::fs::remove_file(&path);
                    let mut file = open_lock_file(&path).with_context(|| {
                        format!(
                            "Failed to reacquire stale plugin operation lock {}",
                            path.display()
                        )
                    })?;
                    let _ = writeln!(file, "{} {}", std::process::id(), plugin_id);
                    return Ok(PluginOperationLock { path });
                }
                anyhow::bail!("Plugin operation already in progress: {}", plugin_id)
            }
            Err(error) => Err(error).with_context(|| {
                format!("Failed to acquire plugin operation lock {}", path.display())
            }),
        }
    }
}

struct PluginOperationLock {
    path: PathBuf,
}

impl Drop for PluginOperationLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn open_lock_file(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
}

fn stale_lockfile(path: &Path) -> bool {
    let Ok(content) = std::fs::read_to_string(path) else {
        return lockfile_too_old(path, LOCKFILE_MAX_AGE);
    };
    let Some(raw_pid) = content.split_whitespace().next() else {
        return lockfile_too_old(path, LOCKFILE_MAX_AGE);
    };
    let Ok(pid) = raw_pid.parse::<u32>() else {
        return lockfile_too_old(path, LOCKFILE_MAX_AGE);
    };

    #[cfg(unix)]
    {
        return !is_pid_alive(pid);
    }

    #[cfg(not(unix))]
    {
        let _ = pid;
        lockfile_too_old(path, LOCKFILE_MAX_AGE)
    }
}

fn lockfile_too_old(path: &Path, max_age: Duration) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    let Ok(modified) = meta.modified() else {
        return false;
    };
    modified.elapsed().is_ok_and(|age| age > max_age)
}

#[cfg(unix)]
fn is_pid_alive(pid: u32) -> bool {
    crate::process_utils::is_pid_alive(pid as i32)
}

fn validate_plugin_id(plugin_id: &str) -> Result<()> {
    if super::validation::is_safe_plugin_id(plugin_id) {
        return Ok(());
    }
    anyhow::bail!("{}: {}", super::validation::INVALID_PLUGIN_ID, plugin_id)
}

fn strip_dev_dependencies_for_release_build(manifest_path: &Path) -> Result<()> {
    let content = std::fs::read_to_string(manifest_path)
        .with_context(|| format!("Failed to read {}", manifest_path.display()))?;
    let mut value: toml::Value = toml::from_str(&content)
        .with_context(|| format!("Failed to parse {}", manifest_path.display()))?;
    let Some(root) = value.as_table_mut() else {
        return Ok(());
    };

    let mut changed = root.remove("dev-dependencies").is_some();
    if let Some(target) = root.get_mut("target").and_then(|v| v.as_table_mut()) {
        for (_, target_entry) in target.iter_mut() {
            if let Some(target_table) = target_entry.as_table_mut() {
                if target_table.remove("dev-dependencies").is_some() {
                    changed = true;
                }
            }
        }
    }

    if changed {
        let rendered = toml::to_string(&value)
            .with_context(|| format!("Failed to render {}", manifest_path.display()))?;
        std::fs::write(manifest_path, rendered)
            .with_context(|| format!("Failed to write {}", manifest_path.display()))?;
        log::info!(
            "Stripped dev-dependencies from {} for install build",
            manifest_path.display()
        );
    }

    Ok(())
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

fn can_build_from_source_fallback(plugin_id: &str, repo: &str, plugin_dir: &Path) -> bool {
    if !plugin_dir.join("Cargo.toml").is_file() {
        return false;
    }

    let expected_repo = format!("qol-tools/{}", plugin_id);
    repo.eq_ignore_ascii_case(&expected_repo)
}

fn dependency_binary_output_path(plugin_dir: &Path, binary_name: &str) -> PathBuf {
    #[cfg(windows)]
    {
        if std::path::Path::new(binary_name).extension().is_none() {
            return plugin_dir.join(format!("{}.exe", binary_name));
        }
    }
    plugin_dir.join(binary_name)
}

fn built_binary_candidates(plugin_dir: &Path, binary_name: &str) -> Vec<PathBuf> {
    let release_dir = plugin_dir.join("target").join("release");
    #[cfg(windows)]
    {
        let mut candidates = vec![release_dir.join(binary_name)];
        if std::path::Path::new(binary_name).extension().is_none() {
            candidates.push(release_dir.join(format!("{}.exe", binary_name)));
        }
        candidates
    }

    #[cfg(not(windows))]
    {
        vec![release_dir.join(binary_name)]
    }
}

async fn install_built_binary(source_path: &Path, output_path: &Path) -> Result<()> {
    let staged_path = output_path.with_extension("new");
    let _ = tokio::fs::remove_file(&staged_path).await;
    tokio::fs::copy(source_path, &staged_path)
        .await
        .with_context(|| {
            format!(
                "Failed to stage built binary {} -> {}",
                source_path.display(),
                staged_path.display()
            )
        })?;
    tokio::fs::rename(&staged_path, output_path)
        .await
        .with_context(|| {
            format!(
                "Failed to install built binary {} -> {}",
                staged_path.display(),
                output_path.display()
            )
        })?;
    Ok(())
}

async fn build_binary_from_source(plugin_dir: &Path, binary_name: &str) -> Result<()> {
    let manifest_path = plugin_dir.join("Cargo.toml");
    if !manifest_path.is_file() {
        anyhow::bail!("Cargo.toml not found at {}", manifest_path.display());
    }

    strip_dev_dependencies_for_release_build(&manifest_path)?;

    let output = run_cargo_build(&manifest_path, plugin_dir).await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Cargo build failed: {}", stderr.trim());
    }

    let source_path = built_binary_candidates(plugin_dir, binary_name)
        .into_iter()
        .find(|path| path.is_file())
        .with_context(|| {
            format!(
                "Built binary not found for {} in {}",
                binary_name,
                plugin_dir.join("target").join("release").display()
            )
        })?;
    let output_path = dependency_binary_output_path(plugin_dir, binary_name);
    install_built_binary(&source_path, &output_path).await?;
    Ok(())
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
    let release: GitHubRelease = github_request(&url).await?.json().await?;

    Ok(release)
}

async fn download_asset(url: &str, dest: &PathBuf) -> Result<()> {
    let response = github_request(url).await?;

    let bytes = response.bytes().await?;
    tokio::fs::write(dest, &bytes).await?;
    Ok(())
}

async fn github_request(url: &str) -> Result<reqwest::Response> {
    let client = reqwest::Client::new();
    let token = super::github::get_stored_token();
    let request = super::github::build_github_request(&client, url, token.as_deref());
    super::github::send_checked(request).await
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
        tokio::fs::create_dir_all(install_dir.join("nested"))
            .await
            .unwrap();
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
        tokio::fs::write(live.join("old.txt"), b"old")
            .await
            .unwrap();
        tokio::fs::write(staging.join("new.txt"), b"new")
            .await
            .unwrap();

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
        tokio::fs::write(live.join("old.txt"), b"old")
            .await
            .unwrap();

        assert!(installer
            .swap_plugin_dirs(&live, &staging, &backup)
            .await
            .is_err());
        assert!(tokio::fs::metadata(live.join("old.txt")).await.is_ok());
        assert!(tokio::fs::metadata(&backup).await.is_err());
    }

    #[test]
    fn strip_dev_dependencies_for_release_build_removes_top_level_and_target_sections() {
        let temp = TempDir::new().unwrap();
        let manifest_path = temp.path().join("Cargo.toml");
        std::fs::write(
            &manifest_path,
            r#"
[package]
name = "sample"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = "1"

[dev-dependencies]
qol-tray = { path = "../../qol-tray" }
toml = "0.9"

[target.'cfg(unix)'.dev-dependencies]
tempfile = "3"
"#,
        )
        .unwrap();

        strip_dev_dependencies_for_release_build(&manifest_path).unwrap();

        let after = std::fs::read_to_string(&manifest_path).unwrap();
        assert!(!after.contains("dev-dependencies"));
        assert!(after.contains("serde"));
    }
}
