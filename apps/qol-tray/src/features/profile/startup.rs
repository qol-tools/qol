use anyhow::Result;
use std::path::Path;

pub(crate) fn run_startup_cleanup(config_dir: &Path) -> Result<()> {
    ensure_profile_dirs_at(config_dir)?;
    migrate_core_file(
        config_dir,
        "hotkeys.json",
        profile_core_path(config_dir, "hotkeys.json"),
    )?;
    migrate_core_file(
        config_dir,
        "shortcuts.json",
        profile_core_path(config_dir, "shortcuts.json"),
    )?;
    migrate_core_file(
        config_dir,
        "task-runner.json",
        profile_core_path(config_dir, "task-runner.json"),
    )?;
    migrate_legacy_plugin_configs(config_dir)?;
    migrate_live_plugin_configs(config_dir)?;
    Ok(())
}

fn migrate_core_file(
    config_dir: &Path,
    legacy_name: &str,
    target_path: std::path::PathBuf,
) -> Result<()> {
    if target_path.exists() {
        return Ok(());
    }
    let legacy_path = config_dir.join(legacy_name);
    if !legacy_path.exists() {
        return Ok(());
    }
    let Some(parent) = target_path.parent() else {
        return Ok(());
    };
    std::fs::create_dir_all(parent)?;
    std::fs::rename(legacy_path, target_path)?;
    Ok(())
}

fn migrate_legacy_plugin_configs(config_dir: &Path) -> Result<()> {
    let legacy_path = config_dir.join("plugin-configs.json");
    if !legacy_path.exists() {
        return Ok(());
    }
    let existing = read_profile_plugin_configs(config_dir)?;
    if !existing.is_empty() {
        let _ = std::fs::remove_file(legacy_path);
        return Ok(());
    }
    let configs = crate::file_io::read_json::<crate::plugins::config::PluginConfigs>(&legacy_path)?;
    for (plugin_id, config) in configs.configs {
        if !crate::paths::is_safe_path_component(&plugin_id) {
            continue;
        }
        crate::file_io::write_pretty_json(
            &profile_plugin_config_path(config_dir, &plugin_id),
            &config,
        )?;
    }
    let _ = std::fs::remove_file(legacy_path);
    Ok(())
}

fn migrate_live_plugin_configs(config_dir: &Path) -> Result<()> {
    let plugins_dir = config_dir.join("plugins");
    let Ok(entries) = std::fs::read_dir(plugins_dir) else {
        return Ok(());
    };
    for entry in entries.filter_map(|entry| entry.ok()) {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(plugin_id) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if !crate::paths::is_safe_path_component(&plugin_id) {
            continue;
        }
        let target_path = profile_plugin_config_path(config_dir, &plugin_id);
        if target_path.exists() {
            continue;
        }
        let live_path = path.join("config.json");
        if !live_path.exists() {
            continue;
        }
        let Ok(config) = crate::file_io::read_json::<serde_json::Value>(&live_path) else {
            continue;
        };
        crate::file_io::write_pretty_json(&target_path, &config)?;
    }
    Ok(())
}

fn ensure_profile_dirs_at(config_dir: &Path) -> Result<()> {
    for path in [
        config_dir.join("profile"),
        config_dir.join("profile/core"),
        config_dir.join("profile/plugin-configs"),
    ] {
        std::fs::create_dir_all(path)?;
    }
    crate::file_io::write_pretty_json(
        &config_dir.join("profile/manifest.json"),
        &crate::features::profile::core::ProfileManifest {
            version: crate::features::profile::core::CURRENT_PROFILE_VERSION,
        },
    )
}

fn profile_core_path(config_dir: &Path, file_name: &str) -> std::path::PathBuf {
    config_dir.join("profile").join("core").join(file_name)
}

fn profile_plugin_config_path(config_dir: &Path, plugin_id: &str) -> std::path::PathBuf {
    config_dir
        .join("profile")
        .join("plugin-configs")
        .join(format!("{}.json", plugin_id))
}

fn read_profile_plugin_configs(config_dir: &Path) -> Result<Vec<String>> {
    let dir = config_dir.join("profile").join("plugin-configs");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(Vec::new());
    };
    Ok(entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_file() {
                return None;
            }
            let plugin_id = path.file_stem()?.to_str()?;
            crate::paths::is_safe_path_component(plugin_id).then(|| plugin_id.to_string())
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    #[test]
    fn migrate_core_files_into_profile_dir() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path();
        std::fs::write(cfg.join("hotkeys.json"), r#"{"hotkeys":[]}"#).unwrap();
        std::fs::write(cfg.join("shortcuts.json"), r#"{"shortcuts":[]}"#).unwrap();
        std::fs::write(cfg.join("task-runner.json"), r#"{"actions":{}}"#).unwrap();

        run_startup_cleanup(cfg).unwrap();

        assert!(!cfg.join("hotkeys.json").exists());
        assert!(!cfg.join("shortcuts.json").exists());
        assert!(!cfg.join("task-runner.json").exists());
        assert!(cfg.join("profile/core/hotkeys.json").exists());
        assert!(cfg.join("profile/core/shortcuts.json").exists());
        assert!(cfg.join("profile/core/task-runner.json").exists());
    }

    #[test]
    fn migrate_legacy_plugin_configs_into_profile_dir() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path();
        let legacy = json!({
            "plugin-launcher": {"enabled": true},
            "plugin-alt-tab": {"monitor": "cursor"}
        });
        std::fs::write(cfg.join("plugin-configs.json"), legacy.to_string()).unwrap();

        run_startup_cleanup(cfg).unwrap();

        assert!(!cfg.join("plugin-configs.json").exists());
        let launcher =
            std::fs::read_to_string(cfg.join("profile/plugin-configs/plugin-launcher.json"))
                .unwrap();
        let alt_tab =
            std::fs::read_to_string(cfg.join("profile/plugin-configs/plugin-alt-tab.json"))
                .unwrap();
        assert!(launcher.contains("enabled"));
        assert!(alt_tab.contains("monitor"));
    }

    #[test]
    fn migrate_live_plugin_configs_into_profile_dir() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path();
        let plugin_dir = cfg.join("plugins/plugin-live");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(
            plugin_dir.join("config.json"),
            json!({"enabled": true, "mode": "cursor"}).to_string(),
        )
        .unwrap();

        run_startup_cleanup(cfg).unwrap();

        let migrated = cfg.join("profile/plugin-configs/plugin-live.json");
        assert!(migrated.exists());
        assert_eq!(
            std::fs::read_to_string(migrated).unwrap(),
            "{\n  \"enabled\": true,\n  \"mode\": \"cursor\"\n}"
        );
    }
}
