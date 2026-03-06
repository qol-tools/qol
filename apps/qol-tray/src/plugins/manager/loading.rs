use super::{autostart, dev_registry, runtime, PluginManager};
use crate::plugins::resolver::{PluginSource, ResolvedPlugin};
use crate::plugins::{Plugin, PluginLoader};
use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;

pub(super) fn load_plugins(manager: &mut PluginManager) -> Result<()> {
    super::super::daemon_tracker::kill_orphan_daemons();
    let loaded = load_all_plugins()?;
    finalize_load(manager, loaded);
    Ok(())
}

struct ResolutionContext {
    resolved: Vec<ResolvedPlugin>,
    resolved_sources: HashMap<String, PluginSource>,
}

struct LoadedPlugins {
    plugins: Vec<Plugin>,
    pids: Vec<u32>,
}

fn load_all_plugins() -> Result<LoadedPlugins> {
    let context = resolution_context()?;
    let mut plugins = PluginLoader::load_resolved(&context.resolved)?;
    super::super::daemon_tracker::clean_stale_sockets(&plugins);
    let pids = autostart::start_plugin_daemons(&mut plugins, &context.resolved_sources);
    Ok(LoadedPlugins { plugins, pids })
}

fn resolution_context() -> Result<ResolutionContext> {
    let plugins_dir = PluginLoader::ensure_plugin_dir()?;
    dev_registry::migrate_symlinks_to_registry(&plugins_dir);
    let resolved = resolve_plugins(&plugins_dir);
    log_resolved_plugins(&resolved);
    let resolved_sources = resolved_sources(&resolved);
    Ok(ResolutionContext {
        resolved,
        resolved_sources,
    })
}

fn resolve_plugins(plugins_dir: &Path) -> Vec<ResolvedPlugin> {
    let dev_links = dev_registry::load_dev_links();
    super::super::resolver::resolve_all(plugins_dir, &dev_links)
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

fn resolved_sources(resolved: &[ResolvedPlugin]) -> HashMap<String, PluginSource> {
    resolved
        .iter()
        .map(|plugin| (plugin.id.clone(), plugin.source.clone()))
        .collect()
}

fn finalize_load(manager: &mut PluginManager, loaded: LoadedPlugins) {
    register_plugins(manager, loaded.plugins);
    super::super::daemon_tracker::save_daemon_pids(&loaded.pids);
    runtime::sync_ignore_pids(manager);
}

fn register_plugins(manager: &mut PluginManager, plugins: Vec<Plugin>) {
    for plugin in plugins {
        manager.plugins.insert(plugin.id.clone(), plugin);
    }
}
