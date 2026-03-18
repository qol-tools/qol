use anyhow::Result;
use std::path::{Path, PathBuf};

const CONFIG_FILE_NAME: &str = "config.json";
const CONFIG_CONTRACT_FILE_NAME: &str = "qol-config.toml";

pub(crate) fn resolve_plugin_root(plugin_id: &str) -> Result<PathBuf> {
    if !crate::paths::is_safe_path_component(plugin_id) {
        anyhow::bail!("Invalid plugin ID: {}", plugin_id);
    }
    let plugins_dir = crate::paths::plugins_dir()?;
    Ok(resolve_plugin_root_from_plugins_dir(
        &plugins_dir,
        plugin_id,
    ))
}

pub(crate) fn resolve_plugin_root_from_plugins_dir(plugins_dir: &Path, plugin_id: &str) -> PathBuf {
    if let Some(dev_path) = resolve_dev_link_path(plugin_id) {
        return dev_path;
    }
    if let Some(worktree_path) = resolve_active_worktree_path(plugin_id) {
        return worktree_path;
    }
    plugins_dir.join(plugin_id)
}

pub(crate) fn config_path(plugin_root: &Path) -> PathBuf {
    plugin_root.join(CONFIG_FILE_NAME)
}

pub(crate) fn config_contract_path(plugin_root: &Path) -> PathBuf {
    plugin_root.join(CONFIG_CONTRACT_FILE_NAME)
}

pub(crate) fn has_custom_ui(plugin_root: &Path) -> bool {
    if plugin_root.join("ui/index.html").exists() {
        return true;
    }
    #[cfg(feature = "dev")]
    if let Some(id) = plugin_root.file_name().and_then(|n| n.to_str()) {
        if let Some(wt) = resolve_active_worktree_path(id) {
            return wt.join("ui/index.html").exists();
        }
    }
    false
}

pub(crate) fn has_config(plugin_root: &Path) -> bool {
    if config_contract_path(plugin_root).exists() {
        return true;
    }
    #[cfg(feature = "dev")]
    if let Some(id) = plugin_root.file_name().and_then(|n| n.to_str()) {
        if let Some(wt) = resolve_active_worktree_path(id) {
            return config_contract_path(&wt).exists();
        }
    }
    false
}

#[cfg(feature = "dev")]
fn resolve_dev_link_path(plugin_id: &str) -> Option<PathBuf> {
    let config_dir = crate::paths::shared_config_dir().ok()?;
    let links = crate::dev::active_dev_links(&config_dir);
    links.get(plugin_id).cloned()
}

#[cfg(feature = "dev")]
fn resolve_active_worktree_path(plugin_id: &str) -> Option<PathBuf> {
    let config_dir = crate::paths::shared_config_dir().ok()?;
    let branch = crate::dev::get_active_worktree_branch(&config_dir)?;
    let worktrees_root = find_ancestor_dir(Path::new(env!("CARGO_MANIFEST_DIR")), "worktrees")?;
    find_branch_worktree(&worktrees_root.join("worktrees"), &branch, plugin_id)
}

#[cfg(feature = "dev")]
fn find_ancestor_dir(start: &Path, dir_name: &str) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|dir| dir.join(dir_name).is_dir())
        .map(|dir| dir.to_path_buf())
}

#[cfg(feature = "dev")]
fn find_branch_worktree(worktrees_root: &Path, branch: &str, plugin_id: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(worktrees_root).ok()?;
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .map(|feature_dir| feature_dir.join(plugin_id))
        .find(|repo_dir| is_matching_branch_worktree(repo_dir, branch))
}

#[cfg(feature = "dev")]
fn is_matching_branch_worktree(repo_dir: &Path, branch: &str) -> bool {
    if !repo_dir.join("plugin.toml").is_file() {
        return false;
    }
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(repo_dir)
        .output()
        .ok();
    let Some(output) = output else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    String::from_utf8_lossy(&output.stdout).trim() == branch
}

#[cfg(not(feature = "dev"))]
fn resolve_dev_link_path(_plugin_id: &str) -> Option<PathBuf> {
    None
}

#[cfg(not(feature = "dev"))]
fn resolve_active_worktree_path(_plugin_id: &str) -> Option<PathBuf> {
    None
}
