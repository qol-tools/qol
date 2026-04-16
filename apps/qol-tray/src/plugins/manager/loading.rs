use super::{autostart, dev_registry, runtime, PluginManager};
use crate::plugins::registry::ensure_registry_initialized;
use crate::plugins::resolver::{
    resolve_from_registry, PluginSource, PluginUnavailable, ResolutionReport, ResolvedPlugin,
};
use crate::plugins::{Plugin, PluginId, PluginLoader};
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::Path;

pub(super) fn load_plugins(manager: &mut PluginManager) -> Result<()> {
    super::super::daemon_tracker::kill_orphan_daemons();
    let loaded = load_all_plugins()?;
    finalize_load(manager, loaded);
    Ok(())
}

struct ResolutionContext {
    report: ResolutionReport,
    resolved_sources: HashMap<PluginId, PluginSource>,
}

struct LoadedPlugins {
    plugins: Vec<Plugin>,
    report: ResolutionReport,
}

fn load_all_plugins() -> Result<LoadedPlugins> {
    let context = resolution_context()?;
    let mut plugins = PluginLoader::load_resolved(&context.report.plugins)?;
    super::super::daemon_tracker::clean_stale_sockets(&plugins);
    autostart::start_plugin_daemons(&mut plugins, &context.resolved_sources);
    Ok(LoadedPlugins {
        plugins,
        report: context.report,
    })
}

fn resolution_context() -> Result<ResolutionContext> {
    let plugins_dir = PluginLoader::ensure_plugin_dir()?;
    dev_registry::migrate_symlinks_to_registry(&plugins_dir);
    let report = resolve_plugins(&plugins_dir)?;
    log_resolved_plugins(&report.plugins);
    let resolved_sources = resolved_sources(&report.plugins);
    Ok(ResolutionContext {
        report,
        resolved_sources,
    })
}

fn resolve_plugins(plugins_dir: &Path) -> Result<ResolutionReport> {
    let config_dir =
        crate::paths::shared_config_dir().context("resolve_plugins: shared_config_dir")?;
    let registry = ensure_registry_initialized(&config_dir, plugins_dir)
        .map_err(|e| anyhow::anyhow!("Failed to initialize plugin registry: {}", e))?;
    let report = resolve_from_registry(&registry);
    log_unavailable(&report.unavailable);
    Ok(report)
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
        log::info!(
            "Resolved plugin: {} ({:?}) from {:?}",
            plugin.id,
            plugin.source,
            plugin.path
        );
    }
}

fn resolved_sources(resolved: &[ResolvedPlugin]) -> HashMap<PluginId, PluginSource> {
    resolved
        .iter()
        .map(|plugin| (plugin.id.clone(), plugin.source.clone()))
        .collect()
}

fn finalize_load(manager: &mut PluginManager, loaded: LoadedPlugins) {
    register_plugins(manager, loaded.plugins);
    manager.set_resolution_report(loaded.report);
    runtime::persist_daemon_pids(manager);
    runtime::sync_ignore_pids(manager);
}

fn register_plugins(manager: &mut PluginManager, plugins: Vec<Plugin>) {
    for plugin in plugins {
        manager.plugins.insert(plugin.id.clone(), plugin);
    }
}
