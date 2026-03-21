use anyhow::{anyhow, Context, Result};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::file_io;

const APP_NAME: &str = "qol-tray";
const INSTALL_ID_ENV: &str = "QOL_TRAY_INSTALL_ID";
const INSTALL_ID_FILE: &str = "qol-tray.install-id";
const ACTIVE_INSTALL_ID_FILE: &str = "active-install-id";

pub const STATE_SOCKET_PATH: &str = "/tmp/qol-tray-state.sock";

pub fn is_safe_path_component(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && !s.starts_with('-')
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn legacy_config_dir() -> Result<PathBuf> {
    dirs::config_dir()
        .context("Could not determine config directory")
        .map(|p| p.join(APP_NAME))
}

pub fn shared_config_dir() -> Result<PathBuf> {
    legacy_config_dir()
}

pub(crate) fn base_data_dir() -> Result<PathBuf> {
    dirs::data_local_dir()
        .or_else(dirs::data_dir)
        .context("Could not determine local data directory")
        .map(|p| p.join(APP_NAME))
}

pub fn installs_dir() -> Result<PathBuf> {
    base_data_dir().map(|p| p.join("installs"))
}

fn valid_install_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn validated_install_id(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    valid_install_id(trimmed).then(|| trimmed.to_string())
}

fn install_id_from_env() -> Option<String> {
    validated_install_id(&env::var(INSTALL_ID_ENV).ok()?)
}

fn install_id_from_marker_file() -> Option<String> {
    let marker_path = env::current_exe().ok()?.parent()?.join(INSTALL_ID_FILE);
    validated_install_id(&fs::read_to_string(marker_path).ok()?)
}

fn active_install_id_path() -> Result<PathBuf> {
    base_data_dir().map(|p| p.join(ACTIVE_INSTALL_ID_FILE))
}

fn install_id_from_active_file() -> Option<String> {
    validated_install_id(&fs::read_to_string(active_install_id_path().ok()?).ok()?)
}

pub fn config_dir_for_install_id(install_id: &str) -> Result<PathBuf> {
    if !valid_install_id(install_id) {
        return Err(anyhow!("invalid install id"));
    }
    installs_dir().map(|p| p.join(install_id))
}

pub fn config_dir() -> Result<PathBuf> {
    if let Some(install_id) = install_id_from_env()
        .or_else(install_id_from_marker_file)
        .or_else(install_id_from_active_file)
    {
        return config_dir_for_install_id(&install_id);
    }
    legacy_config_dir()
}

pub fn set_active_install_id(install_id: &str) -> Result<()> {
    if !valid_install_id(install_id) {
        return Err(anyhow!("invalid install id"));
    }
    let path = active_install_id_path()?;
    file_io::ensure_parent_dir(&path)?;
    fs::write(&path, format!("{}\n", install_id))
        .with_context(|| format!("Failed to write active install marker {}", path.display()))
}

pub fn plugins_dir() -> Result<PathBuf> {
    shared_config_dir().map(|p| p.join("plugins"))
}

pub fn hotkeys_path() -> Result<PathBuf> {
    shared_config_dir().map(|p| p.join("hotkeys.json"))
}

pub fn plugin_configs_path() -> Result<PathBuf> {
    shared_config_dir().map(|p| p.join("plugin-configs.json"))
}

pub fn github_token_path() -> Result<PathBuf> {
    shared_config_dir().map(|p| p.join(".github-token"))
}

pub fn suppressed_errors_path() -> Result<PathBuf> {
    shared_config_dir().map(|p| p.join("suppressed-errors.json"))
}

pub fn plugin_cache_path() -> Result<PathBuf> {
    Ok(runtime_cache_dir().join("plugin-cache.json"))
}

pub fn shortcuts_path() -> Result<PathBuf> {
    shared_config_dir().map(|p| p.join("shortcuts.json"))
}

pub fn task_runner_config_path() -> Result<PathBuf> {
    shared_config_dir().map(|p| p.join("task-runner.json"))
}

#[cfg(feature = "dev")]
pub fn dev_config_path() -> Result<PathBuf> {
    shared_config_dir().map(|p| p.join("dev/config.json"))
}

const RUNTIME_DIR: &str = "/tmp/qol-tray";

pub fn runtime_dir() -> PathBuf {
    PathBuf::from(RUNTIME_DIR)
}

pub fn runtime_pids_dir() -> PathBuf {
    runtime_dir().join("pids")
}

pub fn runtime_cache_dir() -> PathBuf {
    runtime_dir().join("cache")
}

pub fn init_runtime_dirs() -> Result<()> {
    init_runtime_dirs_at(&runtime_dir())
}

fn init_runtime_dirs_at(base: &Path) -> Result<()> {
    if base.exists() {
        fs::remove_dir_all(base)
            .with_context(|| format!("Failed to wipe runtime dir {}", base.display()))?;
    }
    for subdir in ["pids", "cache"] {
        fs::create_dir_all(base.join(subdir))
            .with_context(|| format!("Failed to create runtime subdir {}", subdir))?;
    }
    Ok(())
}

pub fn open_url(url: &str) -> Result<()> {
    open::that(url)?;
    Ok(())
}

pub fn repo_root_from_manifest_dir() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut dir = manifest.as_path();
    loop {
        if dir.join(".worktrees").is_dir() {
            return dir.to_path_buf();
        }
        match dir.parent() {
            Some(parent) => dir = parent,
            None => return manifest,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_have_correct_suffixes() {
        let cases: Vec<(Result<PathBuf>, &str)> = vec![
            (plugins_dir(), "plugins"),
            (hotkeys_path(), "hotkeys.json"),
            (plugin_configs_path(), "plugin-configs.json"),
            (github_token_path(), ".github-token"),
        ];

        let config_path = config_dir().unwrap();
        assert!(
            config_path.to_string_lossy().contains("qol-tray"),
            "config path {:?} should contain qol-tray",
            config_path
        );

        for (result, expected_suffix) in cases {
            let path = result.unwrap();
            assert!(
                path.ends_with(expected_suffix),
                "path {:?} should end with {}",
                path,
                expected_suffix
            );
            if expected_suffix == "plugins" {
                assert!(path.to_string_lossy().contains("qol-tray"));
            }
        }
    }

    #[test]
    fn runtime_dir_is_under_tmp() {
        let dir = runtime_dir();
        assert!(
            dir.starts_with("/tmp"),
            "runtime dir {:?} should be under /tmp",
            dir
        );
        assert!(dir.ends_with("qol-tray"));
    }

    #[test]
    fn runtime_subdirs_have_correct_suffixes() {
        let cases = [(runtime_pids_dir(), "pids"), (runtime_cache_dir(), "cache")];
        for (path, suffix) in cases {
            assert!(
                path.ends_with(suffix),
                "path {:?} should end with {}",
                path,
                suffix
            );
        }
    }

    #[test]
    fn plugin_cache_path_is_under_runtime() {
        let path = plugin_cache_path().unwrap();
        assert!(
            path.starts_with("/tmp/qol-tray"),
            "cache path {:?} should be under /tmp/qol-tray",
            path
        );
    }

    #[test]
    fn init_runtime_dirs_creates_fresh_structure() {
        let test_dir = PathBuf::from("/tmp/qol-tray-test-init");
        let pids = test_dir.join("pids");
        let cache = test_dir.join("cache");

        let _ = std::fs::remove_dir_all(&test_dir);
        std::fs::create_dir_all(&pids).unwrap();
        std::fs::write(pids.join("stale.pid"), "999").unwrap();

        init_runtime_dirs_at(&test_dir).unwrap();

        assert!(pids.is_dir(), "pids dir should exist");
        assert!(cache.is_dir(), "cache dir should exist");
        assert!(
            !pids.join("stale.pid").exists(),
            "stale files should be wiped"
        );

        let _ = std::fs::remove_dir_all(&test_dir);
    }

    #[test]
    fn is_safe_path_component_cases() {
        let valid = [
            "plugin-launcher",
            "my_plugin",
            "plugin123",
            "UPPERCASE",
            "a",
            "MixedCase123",
        ];

        for s in valid {
            assert!(is_safe_path_component(s), "should be valid: {:?}", s);
        }

        let invalid = [
            "../etc",
            "foo/bar",
            "foo\\bar",
            "..",
            ".",
            "",
            "plugin\0evil",
            "plugin/",
            "/plugin",
            "plugin\\",
            "\\plugin",
            "a/b/c",
            "../..",
            "foo/../bar",
            " ",
            ".hidden",
            "..hidden",
            "plugin..name",
            "plugin.name",
            "-plugin",
        ];

        for s in invalid {
            assert!(!is_safe_path_component(s), "should be invalid: {:?}", s);
        }
    }
}
