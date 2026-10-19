use super::{runtime, PluginManager};
use crate::plugins::registry::ensure_registry_initialized;
use crate::plugins::resolver::{
    resolve_effective_registry, PluginUnavailable, ResolutionReport, ResolvedPlugin,
    WorktreeOverride,
};
use crate::plugins::{Plugin, PluginLoader};
use anyhow::{Context, Result};
use std::path::Path;

pub(super) fn load_plugins(manager: &mut PluginManager) -> Result<()> {
    if !crate::dev_generation::is_shadow() && !crate::dev_generation::is_rolling_restart() {
        super::super::daemon_tracker::kill_orphan_daemons();
    }
    let loaded = load_all_plugins()?;
    finalize_load(manager, loaded);
    Ok(())
}

struct LoadedPlugins {
    plugins: Vec<Plugin>,
    report: ResolutionReport,
}

pub(super) struct LoadedPlugin {
    pub(super) plugin: Option<Plugin>,
    pub(super) report: ResolutionReport,
}

fn load_all_plugins() -> Result<LoadedPlugins> {
    let report = resolution_report()?;
    let plugins = PluginLoader::load_resolved(&report.plugins)?;
    if !crate::dev_generation::is_shadow() && !crate::dev_generation::is_rolling_restart() {
        super::super::daemon_tracker::clean_stale_sockets(&plugins);
    }
    Ok(LoadedPlugins { plugins, report })
}

pub(super) fn load_plugin(plugin_id: &str) -> Result<LoadedPlugin> {
    let report = resolution_report()?;
    let plugin = report
        .plugins
        .iter()
        .find(|plugin| plugin.id.as_str() == plugin_id)
        .map(PluginLoader::load_resolved_plugin)
        .transpose()?
        .flatten();
    Ok(LoadedPlugin { plugin, report })
}

fn resolution_report() -> Result<ResolutionReport> {
    let plugins_dir = PluginLoader::ensure_plugin_dir()?;
    let report = resolve_plugins(&plugins_dir)?;
    log_resolution(&report);
    Ok(report)
}

fn resolve_plugins(plugins_dir: &Path) -> Result<ResolutionReport> {
    let config_dir =
        crate::paths::shared_config_dir().context("resolve_plugins: shared_config_dir")?;
    let registry = ensure_registry_initialized(&config_dir, plugins_dir)
        .map_err(|e| anyhow::anyhow!("Failed to initialize plugin registry: {}", e))?;
    Ok(resolve_effective_registry(&registry, &config_dir))
}

fn log_resolution(report: &ResolutionReport) {
    log_worktree_override(&report.worktree);
    log_unavailable(&report.unavailable);
    log_resolved_plugins(&report.plugins);
}

fn log_worktree_override(worktree: &WorktreeOverride) {
    for id in &worktree.overridden {
        log::info!("[worktree] overriding plugin={id} with active worktree slot");
    }
    for id in &worktree.promoted {
        log::info!("[worktree] detaching dev link for {id}: promoting installed fallback");
    }
    if !worktree.removed.is_empty() {
        log::info!(
            "[worktree] detaching dev links outside the active selection: {}",
            worktree.removed.join(", ")
        );
    }
}

fn log_unavailable(unavailable: &[PluginUnavailable]) {
    for u in unavailable {
        let fallback_info = match &u.fallback {
            Some(f) => format!(" (fallback {} also failed: {})", f.path.display(), f.reason),
            None => " (no fallback)".to_string(),
        };
        log::warn!(
            "Plugin {} unavailable: active {} failed — {}{}",
            u.id,
            u.active.path.display(),
            u.active.reason,
            fallback_info
        );
    }
}

fn log_resolved_plugins(resolved: &[ResolvedPlugin]) {
    for plugin in resolved {
        if let Some(failure) = &plugin.active_failure {
            log::warn!(
                "Plugin {} active slot invalid ({}); falling back to {}",
                plugin.id,
                failure.reason,
                plugin.path.display()
            );
        }
        log::info!(
            "Resolved plugin: {} ({:?}) from {:?}",
            plugin.id,
            plugin.source,
            plugin.path
        );
    }
}

fn finalize_load(manager: &mut PluginManager, loaded: LoadedPlugins) {
    register_plugins(manager, loaded.plugins);
    manager.set_resolution_report(loaded.report);
    runtime::sync_ignore_pids(manager);
}

fn register_plugins(manager: &mut PluginManager, plugins: Vec<Plugin>) {
    for plugin in plugins {
        manager.plugins.insert(plugin.id.clone(), plugin);
    }
    rebuild_identity_index(manager);
}

pub(super) fn rebuild_identity_index(manager: &mut PluginManager) {
    use crate::plugins::PluginIdentityIndex;
    let mut index = PluginIdentityIndex::default();
    for plugin in manager.plugins.values() {
        index.insert(
            plugin.uid(),
            plugin.id.clone(),
            plugin.manifest.plugin.name.clone(),
        );
    }
    manager.identity_index = index;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::resolver::{PluginSource, ResolutionOrigin, SlotFailure};
    use crate::plugins::PluginId;
    use std::path::PathBuf;

    #[test]
    fn a_load_pass_logs_every_worktree_change_and_fallback_once() {
        let report = ResolutionReport {
            plugins: vec![ResolvedPlugin {
                id: PluginId::new("plugin-foo".to_string()),
                path: PathBuf::from("/a/installed/plugin-foo"),
                source: PluginSource::Installed,
                resolved_from: ResolutionOrigin::Fallback,
                active_failure: Some(SlotFailure {
                    path: PathBuf::from("/a/deleted/plugin-foo"),
                    reason: "not a directory: /a/deleted/plugin-foo".to_string(),
                }),
            }],
            unavailable: Vec::new(),
            worktree: WorktreeOverride {
                overridden: vec!["plugin-bar".to_string()],
                promoted: vec!["plugin-baz".to_string()],
                removed: vec!["plugin-qux".to_string(), "plugin-quux".to_string()],
            },
        };

        let ((), logs) =
            crate::test_support::capture_logs(log::Level::Info, || log_resolution(&report));

        assert_eq!(
            logs,
            [
                "INFO [worktree] overriding plugin=plugin-bar with active worktree slot",
                "INFO [worktree] detaching dev link for plugin-baz: promoting installed fallback",
                "INFO [worktree] detaching dev links outside the active selection: plugin-qux, plugin-quux",
                "WARN Plugin plugin-foo active slot invalid (not a directory: /a/deleted/plugin-foo); falling back to /a/installed/plugin-foo",
                "INFO Resolved plugin: plugin-foo (Installed) from \"/a/installed/plugin-foo\"",
            ]
        );
    }
}
