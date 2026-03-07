use std::collections::HashMap;
use std::path::Path;

use super::LinkedPlugin;

pub fn list_linked_plugins(config_dir: &Path) -> Result<Vec<LinkedPlugin>, String> {
    let links = super::store::load_dev_links(config_dir);
    let known_fingerprints = crate::dev::load_build_fingerprints(config_dir);
    let plans = super::super::build::plan_linked_plugin_builds(&links, &known_fingerprints);
    let log_controls = crate::logging::load_all_plugin_controls(config_dir);
    let plans_by_id: HashMap<String, _> = plans
        .into_iter()
        .map(|p| (p.plugin_id.clone(), p))
        .collect();
    let mut plugins: Vec<LinkedPlugin> = links
        .iter()
        .map(|(id, path)| {
            let name = read_plugin_name(&path.join("plugin.toml")).unwrap_or_else(|| id.clone());
            let log_control = log_controls.get(id).cloned().unwrap_or_default();
            build_plugin_entry(id, path, plans_by_id.get(id), name, log_control)
        })
        .collect();
    plugins.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(plugins)
}

fn build_plugin_entry(
    id: &str,
    path: &Path,
    plan: Option<&crate::dev::build::PluginBuildPlan>,
    name: String,
    log_control: crate::logging::LogControl,
) -> LinkedPlugin {
    LinkedPlugin {
        id: id.to_string(),
        name,
        source: path.to_string_lossy().to_string(),
        has_cargo: plan.map(|p| p.has_cargo).unwrap_or(false),
        supports_platform: plan.map(|p| p.supports_platform).unwrap_or(true),
        needs_rebuild: plan.map(|p| p.needs_rebuild).unwrap_or(false),
        rebuild_reason: plan
            .map(|p| p.reason.clone())
            .unwrap_or_else(|| "Unknown".to_string()),
        fingerprint: plan.and_then(|p| p.current_fingerprint.clone()),
        last_built_fingerprint: plan.and_then(|p| p.last_built_fingerprint.clone()),
        logs_muted: log_control.muted,
        suppressed_log_patterns: log_control.suppress_patterns,
    }
}

fn read_plugin_name(toml_path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(toml_path).ok()?;
    let manifest: crate::plugins::PluginManifest = toml::from_str(&content).ok()?;
    Some(manifest.plugin.name)
}
