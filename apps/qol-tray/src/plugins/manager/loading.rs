use super::{autostart, runtime, PluginManager};
use crate::plugins::registry::ensure_registry_initialized;
use crate::plugins::resolver::{
    resolve_from_registry, PluginSource, PluginUnavailable, ResolutionReport, ResolvedPlugin,
};
use crate::plugins::{Plugin, PluginId, PluginLoader};
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::Path;
#[cfg(feature = "dev")]
use std::path::PathBuf;

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
    let mut registry = ensure_registry_initialized(&config_dir, plugins_dir)
        .map_err(|e| anyhow::anyhow!("Failed to initialize plugin registry: {}", e))?;
    apply_worktree_override(&mut registry, &config_dir);
    let report = resolve_from_registry(&registry);
    log_unavailable(&report.unavailable);
    Ok(report)
}

#[cfg(feature = "dev")]
fn apply_worktree_override(
    registry: &mut crate::plugins::registry::Registry,
    config_dir: &Path,
) {
    use crate::plugins::registry::SlotSource;
    let branch = crate::dev::get_active_worktree_branch(config_dir);
    let Some(branch) = branch else { return };
    let base_links: HashMap<String, PathBuf> = registry
        .entries
        .iter()
        .filter(|e| {
            matches!(
                e.active.source,
                SlotSource::DevLink { .. } | SlotSource::WorktreeLink { .. }
            )
        })
        .map(|e| (e.id.clone(), e.active.path.clone()))
        .collect();
    let resolved = crate::dev::resolve_worktree_paths(&base_links, Some(&branch));
    for entry in &mut registry.entries {
        if let Some(new_path) = resolved.get(&entry.id) {
            if *new_path != entry.active.path {
                log::info!(
                    "[worktree] overriding {} path: {} → {}",
                    entry.id,
                    entry.active.path.display(),
                    new_path.display()
                );
                entry.active.path = new_path.clone();
            }
        }
    }
}

#[cfg(not(feature = "dev"))]
fn apply_worktree_override(
    _registry: &mut crate::plugins::registry::Registry,
    _config_dir: &Path,
) {
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
