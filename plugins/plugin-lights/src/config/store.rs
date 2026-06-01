use anyhow::{Context, Error, Result};
use std::path::PathBuf;

use super::model::PluginConfig;

const PLUGIN_NAMES: &[&str] = &["plugin-lights"];

pub fn load() -> Result<PluginConfig> {
    let path = existing_config_path();
    if path.is_none() {
        let config = PluginConfig::default();
        save(&config)?;
        return Ok(config);
    }

    let path = path.unwrap();
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let config: PluginConfig = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    super::validation::validate(&config).map_err(Error::msg)?;
    migrate_legacy_config(&config, &path)?;
    Ok(config)
}

pub fn save(config: &PluginConfig) -> Result<()> {
    super::validation::validate(config).map_err(Error::msg)?;
    let json = serde_json::to_string_pretty(config).context("failed to serialize config")?;
    let content = format!("{}\n", json);

    let path = writable_config_path()?;
    write_config(&path, &content)?;
    Ok(())
}

fn migrate_legacy_config(config: &PluginConfig, source: &PathBuf) -> Result<()> {
    let target = writable_config_path()?;
    if *source == target {
        return Ok(());
    }

    let json = serde_json::to_string_pretty(config).context("failed to serialize config")?;
    let content = format!("{}\n", json);
    write_config(&target, &content)
}

fn write_config(path: &PathBuf, content: &str) -> Result<()> {
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    std::fs::write(path, content).with_context(|| format!("failed to write {}", path.display()))
}

fn existing_config_path() -> Option<PathBuf> {
    config_paths().into_iter().find(|path| path.is_file())
}

fn writable_config_path() -> Result<PathBuf> {
    preferred_write_path(
        qol_config::plugin_config_paths(PLUGIN_NAMES),
        legacy_config_path(),
    )
    .context("plugin-lights could not resolve a writable config path")
}

fn config_paths() -> Vec<PathBuf> {
    merged_config_paths(
        qol_config::plugin_config_paths(PLUGIN_NAMES),
        legacy_config_path(),
    )
}

fn merged_config_paths(mut paths: Vec<PathBuf>, legacy_path: Option<PathBuf>) -> Vec<PathBuf> {
    if let Some(legacy_path) = legacy_path {
        if !paths.contains(&legacy_path) {
            paths.push(legacy_path);
        }
    }
    paths
}

fn preferred_write_path(paths: Vec<PathBuf>, legacy_path: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(path) = paths.into_iter().next() {
        return Some(path);
    }

    legacy_path
}

fn legacy_config_path() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(
        PathBuf::from(home)
            .join(".config")
            .join("qol-tray")
            .join("plugins")
            .join("plugin-lights")
            .join("config.json"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merged_config_paths_appends_legacy_when_missing() {
        let primary = PathBuf::from("/primary/config.json");
        let legacy = PathBuf::from("/legacy/config.json");
        let paths = merged_config_paths(vec![primary.clone()], Some(legacy.clone()));
        assert_eq!(paths, vec![primary, legacy]);
    }

    #[test]
    fn merged_config_paths_deduplicates_legacy_path() {
        let primary = PathBuf::from("/primary/config.json");
        let paths = merged_config_paths(vec![primary.clone()], Some(primary.clone()));
        assert_eq!(paths, vec![primary]);
    }

    #[test]
    fn preferred_write_path_falls_back_to_legacy_when_shared_paths_missing() {
        let legacy = PathBuf::from("/legacy/config.json");
        let resolved = preferred_write_path(Vec::new(), Some(legacy.clone()));
        assert_eq!(resolved, Some(legacy));
    }
}
