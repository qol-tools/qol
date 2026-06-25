use super::{runtime, PluginManager};
use crate::plugins::registry::ensure_registry_initialized;
use crate::plugins::resolver::{
    resolve_from_registry, PluginUnavailable, ResolutionReport, ResolvedPlugin,
};
use crate::plugins::{Plugin, PluginLoader};
use anyhow::{Context, Result};
#[cfg(feature = "dev")]
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

struct LoadedPlugins {
    plugins: Vec<Plugin>,
    report: ResolutionReport,
}

fn load_all_plugins() -> Result<LoadedPlugins> {
    let report = resolution_report()?;
    let plugins = PluginLoader::load_resolved(&report.plugins)?;
    super::super::daemon_tracker::clean_stale_sockets(&plugins);
    Ok(LoadedPlugins { plugins, report })
}

fn resolution_report() -> Result<ResolutionReport> {
    let plugins_dir = PluginLoader::ensure_plugin_dir()?;
    let report = resolve_plugins(&plugins_dir)?;
    log_resolved_plugins(&report.plugins);
    Ok(report)
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
fn apply_worktree_override(registry: &mut crate::plugins::registry::Registry, config_dir: &Path) {
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
            override_entry_to_worktree(entry, new_path);
        }
    }
}

/// Repoint an entry's active slot at the worktree, making the pre-override slot
/// the fallback so a plugin whose binary was not built in the worktree resolves
/// to its main-clone build. The override only runs on dev-linked actives, so the
/// pre-override slot is always the main-clone build; it replaces any stale
/// installed-release fallback (which would be protocol-incompatible with a
/// freshly built host).
#[cfg(feature = "dev")]
fn override_entry_to_worktree(entry: &mut crate::plugins::registry::Entry, new_path: &Path) {
    if *new_path == entry.active.path {
        return;
    }
    log::info!(
        "[worktree] overriding {} path: {} → {}",
        entry.id,
        entry.active.path.display(),
        new_path.display()
    );
    entry.fallback = Some(entry.active.clone());
    entry.active.path = new_path.to_path_buf();
}

#[cfg(not(feature = "dev"))]
fn apply_worktree_override(_registry: &mut crate::plugins::registry::Registry, _config_dir: &Path) {
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

fn finalize_load(manager: &mut PluginManager, loaded: LoadedPlugins) {
    register_plugins(manager, loaded.plugins);
    manager.set_resolution_report(loaded.report);
    runtime::sync_ignore_pids(manager);
}

fn register_plugins(manager: &mut PluginManager, plugins: Vec<Plugin>) {
    use crate::plugins::PluginIdentityIndex;
    let mut index = PluginIdentityIndex::default();
    for plugin in plugins {
        index.insert(
            plugin.uid(),
            plugin.id.clone(),
            plugin.manifest.plugin.name.clone(),
        );
        manager.plugins.insert(plugin.id.clone(), plugin);
    }
    manager.identity_index = index;
}

#[cfg(all(test, feature = "dev"))]
mod worktree_override_tests {
    use super::override_entry_to_worktree;
    use crate::plugins::registry::{Entry, Slot, SlotSource};
    use std::path::PathBuf;

    fn dev_linked(active: &str, fallback: Option<&str>) -> Entry {
        Entry {
            id: "plugin-foo".to_string(),
            active: Slot {
                path: PathBuf::from(active),
                source: SlotSource::DevLink {
                    origin_path: PathBuf::from(active),
                },
            },
            fallback: fallback.map(|p| Slot {
                path: PathBuf::from(p),
                source: SlotSource::ReleaseAsset,
            }),
        }
    }

    #[test]
    fn override_resolves_active_and_fallback() {
        let cases = [
            (
                "missing fallback is filled from prior active",
                None,
                "/wt",
                "/wt",
                Some("/main"),
            ),
            (
                "stale installed fallback is replaced by main clone",
                Some("/rel"),
                "/wt",
                "/wt",
                Some("/main"),
            ),
            ("no-op when path is unchanged", None, "/main", "/main", None),
        ];
        for (label, initial_fallback, new_path, want_active, want_fallback) in cases {
            let mut entry = dev_linked("/main", initial_fallback);
            override_entry_to_worktree(&mut entry, &PathBuf::from(new_path));
            assert_eq!(
                entry.active.path,
                PathBuf::from(want_active),
                "active path: {label}"
            );
            assert_eq!(
                entry.fallback.as_ref().map(|s| s.path.clone()),
                want_fallback.map(PathBuf::from),
                "fallback path: {label}"
            );
        }
    }

    #[test]
    fn filled_fallback_is_the_original_main_clone_dev_link() {
        let mut entry = dev_linked("/main", None);
        override_entry_to_worktree(&mut entry, &PathBuf::from("/wt"));
        let fallback = entry.fallback.expect("fallback populated");
        assert!(
            matches!(fallback.source, SlotSource::DevLink { .. }),
            "fallback keeps the original dev-link source so resolution targets the main clone"
        );
    }
}
