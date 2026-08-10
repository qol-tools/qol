#![cfg(feature = "dev")]

use std::sync::Arc;

use crate::daemon::ConfigKind;
use crate::dev;

use super::super::dev_runtime::DevRuntimeService;
use super::super::helpers::shared_config_dir;
use super::super::types::AppState;

pub(super) fn queue_reload(
    state: &AppState,
    worktree_branch: Option<String>,
) -> Result<(), &'static str> {
    let runtime = state.runtime.clone();
    if !runtime.try_start_build() {
        return Err("Build already in progress");
    }

    log::info!("Developer reload requested");
    if worktree_branch.is_some() {
        if let Err(error) =
            super::super::helpers::persist_worktree_branch(worktree_branch.as_deref())
        {
            runtime.finish_build();
            log::error!("Failed to persist worktree selection: {error}");
            return Err("Failed to persist worktree selection");
        }
        super::refresh_discovery(state);
    }

    let task = reload_task(state, runtime.clone(), None, worktree_branch);
    tokio::task::spawn_blocking(move || run_reload(task));
    Ok(())
}

pub(super) fn queue_reload_single(state: &AppState, plugin_id: String) -> Result<(), &'static str> {
    let runtime = state.runtime.clone();
    if !runtime.try_start_build() {
        return Err("Build already in progress");
    }

    log::info!("Developer reload requested for plugin: {}", plugin_id);
    let branch = shared_config_dir()
        .ok()
        .and_then(|dir| dev::get_active_worktree_branch(&dir));
    let task = reload_task(state, runtime.clone(), Some(plugin_id), branch);
    tokio::task::spawn_blocking(move || run_reload(task));
    Ok(())
}

struct ReloadTask {
    runtime: Arc<DevRuntimeService>,
    plugin_manager: std::sync::Arc<std::sync::Mutex<crate::plugins::PluginManager>>,
    config: std::sync::Arc<crate::daemon::ConfigBus>,
    events: std::sync::Arc<crate::daemon::EventBus>,
    config_dir: Option<std::path::PathBuf>,
    plugin_filter: Option<String>,
    worktree_branch: Option<String>,
}

struct BuildGuard {
    runtime: Arc<DevRuntimeService>,
}

impl Drop for BuildGuard {
    fn drop(&mut self) {
        self.runtime.finish_build();
    }
}

fn reload_task(
    state: &AppState,
    runtime: Arc<DevRuntimeService>,
    plugin_filter: Option<String>,
    worktree_branch: Option<String>,
) -> ReloadTask {
    ReloadTask {
        runtime,
        plugin_manager: state.plugin_manager.clone(),
        config: state.daemon.config.clone(),
        events: state.daemon.events.clone(),
        config_dir: shared_config_dir().ok(),
        plugin_filter,
        worktree_branch,
    }
}

fn run_reload(task: ReloadTask) {
    let _guard = BuildGuard {
        runtime: task.runtime.clone(),
    };
    run_build(&task);
    let plugin_filter = task.plugin_filter.clone();
    reload_plugins(
        task.plugin_manager,
        task.config,
        task.events,
        plugin_filter.as_deref(),
    );
    log_plugin_staleness();
}

fn log_plugin_staleness() {
    let report = crate::doctor::check_single("plugin_staleness");
    for outcome in report
        .outcomes()
        .filter(|outcome| !matches!(outcome.status, crate::doctor::OutcomeStatus::Ok))
    {
        log::warn!(
            "Developer reload plugin staleness remains: {}",
            outcome.message
        );
    }
}

fn run_build(task: &ReloadTask) {
    let dev_links: std::collections::HashMap<String, std::path::PathBuf> = task
        .config_dir
        .as_deref()
        .map(crate::plugins::registry::dev_linked_paths)
        .unwrap_or_default()
        .into_iter()
        .filter(|(k, _)| task.plugin_filter.as_ref().is_none_or(|id| k == id))
        .collect();

    let persisted_branch = task
        .config_dir
        .as_deref()
        .and_then(dev::get_active_worktree_branch);
    let branch = task
        .worktree_branch
        .as_deref()
        .or(persisted_branch.as_deref());

    let event_sink = task.runtime.create_core_event_sink(task.events.clone());
    let build_service = dev::default_build_application_service(event_sink.as_ref());
    build_service.run(&dev_links, task.config_dir.as_deref(), branch);
}

fn reload_plugins(
    plugin_manager: std::sync::Arc<std::sync::Mutex<crate::plugins::PluginManager>>,
    config: std::sync::Arc<crate::daemon::ConfigBus>,
    events: std::sync::Arc<crate::daemon::EventBus>,
    plugin_filter: Option<&str>,
) {
    let mut manager = match plugin_manager.lock() {
        Ok(manager) => manager,
        Err(error) => {
            log::error!("Plugin manager mutex poisoned: {}", error);
            return;
        }
    };

    let reload_result = match plugin_filter {
        Some(plugin_id) => manager.reload_plugin(plugin_id),
        None => manager.reload_plugins(),
    };
    if let Err(error) = reload_result {
        if let Some(plugin_id) = plugin_filter {
            log::error!("Failed to reload plugin {}: {}", plugin_id, error);
        } else {
            log::error!("Failed to reload plugins: {}", error);
        }
        return;
    }
    drop(manager);

    match plugin_filter {
        Some(plugin_id) => log::info!("Plugin {} reloaded successfully", plugin_id),
        None => log::info!("Plugins reloaded successfully"),
    }
    config.config_changed(ConfigKind::Plugins);
    events.send_plugins_changed();
}
