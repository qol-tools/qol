#![cfg(feature = "dev")]

use std::sync::Arc;

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
    super::super::helpers::persist_worktree_branch(worktree_branch.as_deref());

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
    let task = reload_task(state, runtime.clone(), Some(plugin_id), None);
    tokio::task::spawn_blocking(move || run_reload(task));
    Ok(())
}

struct ReloadTask {
    runtime: Arc<DevRuntimeService>,
    plugin_manager: std::sync::Arc<std::sync::Mutex<crate::plugins::PluginManager>>,
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
    reload_plugins(task.plugin_manager, task.events);
}

fn run_build(task: &ReloadTask) {
    let dev_links: std::collections::HashMap<String, std::path::PathBuf> = task
        .config_dir
        .as_deref()
        .map(dev::load_dev_links)
        .unwrap_or_default()
        .into_iter()
        .filter(|(k, _)| task.plugin_filter.as_ref().is_none_or(|id| k == id))
        .collect();

    let event_sink = task.runtime.create_core_event_sink(task.events.clone());
    let build_service = dev::default_build_application_service(event_sink.as_ref());
    build_service.run(
        &dev_links,
        task.config_dir.as_deref(),
        task.worktree_branch.as_deref(),
    );
}

fn reload_plugins(
    plugin_manager: std::sync::Arc<std::sync::Mutex<crate::plugins::PluginManager>>,
    events: std::sync::Arc<crate::daemon::EventBus>,
) {
    let mut manager = match plugin_manager.lock() {
        Ok(manager) => manager,
        Err(error) => {
            log::error!("Plugin manager mutex poisoned: {}", error);
            return;
        }
    };

    if let Err(error) = manager.reload_plugins() {
        log::error!("Failed to reload plugins: {}", error);
        return;
    }

    log::info!("Plugins reloaded successfully");
    crate::hotkeys::trigger_reload();
    events.send_plugins_changed();
}
