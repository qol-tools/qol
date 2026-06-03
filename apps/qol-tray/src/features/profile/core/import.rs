use super::storage::write_core_settings;
use super::{
    ApplyProfileResult, ImportPluginResult, PluginLockEntry, PluginsLock, ProfileImportBundle,
    CURRENT_PROFILE_VERSION,
};
use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

pub async fn apply_import_bundle(
    plugins_dir: &Path,
    bundle: &ProfileImportBundle,
) -> Result<ApplyProfileResult> {
    super::ensure_profile_dirs()?;
    let previous_lock = super::load_plugins_lock().unwrap_or(PluginsLock {
        version: CURRENT_PROFILE_VERSION,
        plugins: Vec::new(),
    });
    let plugins = super::import_plugins(bundle);
    validate_imported_plugin_configs(plugins_dir, bundle.plugin_configs.as_ref(), &plugins).await?;
    let plugin_results = reconcile_plugins(plugins_dir, &plugins).await;
    write_core_settings(bundle)?;
    if let Some(plugin_configs) = &bundle.plugin_configs {
        super::replace_plugin_configs(plugin_configs)?;
    }
    project_plugin_configs_to_dir(plugins_dir, bundle.plugin_configs.as_ref())?;
    super::plugins_lock::sync_plugins_lock_from_imported_state(
        plugins_dir,
        &previous_lock,
        &plugins,
    )?;
    let success = plugin_results
        .iter()
        .all(|result| result.status != "failed");

    Ok(ApplyProfileResult {
        success,
        plugins: plugin_results,
    })
}

async fn reconcile_plugins(
    plugins_dir: &Path,
    plugins: &[PluginLockEntry],
) -> Vec<ImportPluginResult> {
    let installer =
        crate::features::plugin_store::installer::PluginInstaller::new(plugins_dir.to_path_buf());
    let mut results = Vec::new();

    for plugin in plugins {
        if !crate::plugins::manifest::supports_current_platform(&plugin.platforms) {
            results.push(ImportPluginResult {
                id: plugin.id.clone(),
                status: "skipped".to_string(),
                message: "unsupported on this platform".to_string(),
            });
            continue;
        }

        let plugin_dir = plugins_dir.join(&plugin.id);
        let current_version = super::plugins_lock::read_plugin_version(&plugin_dir).ok();
        if current_version.as_deref() == Some(plugin.version.as_str()) && !plugin.version.is_empty()
        {
            results.push(ImportPluginResult {
                id: plugin.id.clone(),
                status: "kept".to_string(),
                message: format!("already at {}", plugin.version),
            });
            continue;
        }

        results.push(restore_plugin(&installer, &plugin_dir, plugin).await);
    }

    results
}

async fn restore_plugin(
    installer: &crate::features::plugin_store::installer::PluginInstaller,
    plugin_dir: &Path,
    plugin: &PluginLockEntry,
) -> ImportPluginResult {
    let exists = plugin_dir.exists();
    let action = action_for_install(exists);
    let Some(source) = crate::features::plugin_store::source::resolve_source_for_plugin(&plugin.id)
    else {
        return ImportPluginResult {
            id: plugin.id.clone(),
            status: "failed".to_string(),
            message: format!("{action} failed: no plugin source provides {}", plugin.id),
        };
    };

    let result = if plugin.version.is_empty() {
        restore_latest(installer, &source, &plugin.id, exists).await
    } else {
        restore_exact(installer, &source, &plugin.id, &plugin.version, exists).await
    };

    if let Some(error) = result.err() {
        return ImportPluginResult {
            id: plugin.id.clone(),
            status: "failed".to_string(),
            message: format!("{action} failed: {error:#}"),
        };
    }

    ImportPluginResult {
        id: plugin.id.clone(),
        status: action.to_string(),
        message: plugin_restore_message(plugin, action),
    }
}

fn action_for_install(exists: bool) -> &'static str {
    if exists {
        return "update";
    }
    "install"
}

async fn restore_latest(
    installer: &crate::features::plugin_store::installer::PluginInstaller,
    source: &crate::features::plugin_store::source::PluginSource,
    plugin_id: &str,
    exists: bool,
) -> Result<()> {
    if exists {
        return installer.update(source, plugin_id).await;
    }
    installer.install(source, plugin_id).await
}

async fn restore_exact(
    installer: &crate::features::plugin_store::installer::PluginInstaller,
    source: &crate::features::plugin_store::source::PluginSource,
    plugin_id: &str,
    version: &str,
    exists: bool,
) -> Result<()> {
    if exists {
        return installer.update_exact(source, plugin_id, version).await;
    }
    installer.install_exact(source, plugin_id, version).await
}

fn plugin_restore_message(plugin: &PluginLockEntry, action: &str) -> String {
    let verb = if action == "update" {
        "updated"
    } else {
        "installed"
    };
    if plugin.version.is_empty() {
        return format!("{verb} latest available version");
    }
    format!("{verb} {}", plugin.version)
}

fn project_plugin_configs_to_dir(
    plugins_dir: &Path,
    plugin_configs: Option<&HashMap<String, Value>>,
) -> Result<()> {
    let Some(plugin_configs) = plugin_configs else {
        return Ok(());
    };
    remove_live_plugin_configs_missing_from_profile(plugins_dir, plugin_configs)?;
    let manager = crate::plugins::PluginConfigManager::new()?;
    for (plugin_id, config) in plugin_configs {
        if !plugins_dir.join(plugin_id).is_dir() {
            continue;
        }
        manager.set_config(plugin_id, config.clone())?;
    }
    Ok(())
}

async fn validate_imported_plugin_configs(
    plugins_dir: &Path,
    plugin_configs: Option<&HashMap<String, Value>>,
    requested_plugins: &[PluginLockEntry],
) -> Result<()> {
    let Some(plugin_configs) = plugin_configs else {
        return Ok(());
    };
    let requested_plugins = requested_plugins
        .iter()
        .map(|plugin| (plugin.id.as_str(), plugin))
        .collect::<HashMap<_, _>>();
    let installer =
        crate::features::plugin_store::installer::PluginInstaller::new(plugins_dir.to_path_buf());
    let mut plugin_ids = plugin_configs.keys().cloned().collect::<Vec<_>>();
    plugin_ids.sort();
    for plugin_id in plugin_ids {
        let requested_plugin = requested_plugins.get(plugin_id.as_str()).copied();
        let Some(spec) =
            load_validation_contract(&installer, plugins_dir, &plugin_id, requested_plugin).await?
        else {
            continue;
        };
        let config = plugin_configs
            .get(&plugin_id)
            .context("missing plugin config")?;
        let errors = match crate::plugins::config::validate_config_value(&spec, config) {
            Ok(()) => continue,
            Err(errors) => errors,
        };
        anyhow::bail!(
            "Invalid config for {}: {}",
            plugin_id,
            crate::plugins::config::format_validation_errors(errors)
        );
    }
    Ok(())
}

async fn load_validation_contract(
    installer: &crate::features::plugin_store::installer::PluginInstaller,
    plugins_dir: &Path,
    plugin_id: &str,
    requested_plugin: Option<&PluginLockEntry>,
) -> Result<Option<qol_config::contract::ConfigSpec>> {
    let current_version =
        super::plugins_lock::read_plugin_version(&plugins_dir.join(plugin_id)).ok();
    if should_use_installed_contract(current_version.as_deref(), requested_plugin) {
        let spec = crate::plugins::config::load_config_contract(plugin_id)?;
        if spec.is_some() {
            return Ok(spec);
        }
    }
    let Some(requested_plugin) = requested_plugin else {
        return Ok(None);
    };
    let Some(source) =
        crate::features::plugin_store::source::resolve_source_for_plugin(&requested_plugin.id)
    else {
        anyhow::bail!(
            "No plugin source provides {} for contract validation",
            requested_plugin.id
        );
    };
    installer
        .load_source_config_contract(
            &source,
            requested_plugin.id.as_str(),
            requested_plugin.version_option(),
        )
        .await
        .with_context(|| format!("Failed to load config contract for {}", requested_plugin.id))
}

fn should_use_installed_contract(
    current_version: Option<&str>,
    requested_plugin: Option<&PluginLockEntry>,
) -> bool {
    let Some(requested_plugin) = requested_plugin else {
        return true;
    };
    if requested_plugin.version.is_empty() {
        return current_version.is_some();
    }
    current_version == Some(requested_plugin.version.as_str())
}

impl PluginLockEntry {
    fn version_option(&self) -> Option<&str> {
        if self.version.is_empty() {
            return None;
        }
        Some(self.version.as_str())
    }
}

fn remove_live_plugin_configs_missing_from_profile(
    plugins_dir: &Path,
    plugin_configs: &HashMap<String, Value>,
) -> Result<()> {
    let Ok(entries) = std::fs::read_dir(plugins_dir) else {
        return Ok(());
    };
    for entry in entries.filter_map(|entry| entry.ok()) {
        let path = entry.path();
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            continue;
        }
        let Ok(plugin_id) = entry.file_name().into_string() else {
            continue;
        };
        if !crate::paths::is_safe_path_component(&plugin_id) {
            continue;
        }
        if plugin_configs.contains_key(&plugin_id) {
            continue;
        }
        let config_path = crate::plugins::paths::config_path(&path);
        if !config_path.exists() {
            continue;
        }
        std::fs::remove_file(config_path)?;
    }
    Ok(())
}
