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
    let build_run = run_build(&task);
    let plugin_filter = task.plugin_filter.clone();
    reload_plugins(
        task.plugin_manager,
        task.config,
        task.events,
        plugin_filter.as_deref(),
        &build_run,
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

fn run_build(task: &ReloadTask) -> crate::dev::BuildRun {
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
    build_service.run(&dev_links, task.config_dir.as_deref(), branch)
}

fn reload_plugins(
    plugin_manager: std::sync::Arc<std::sync::Mutex<crate::plugins::PluginManager>>,
    config: std::sync::Arc<crate::daemon::ConfigBus>,
    events: std::sync::Arc<crate::daemon::EventBus>,
    plugin_filter: Option<&str>,
    build_run: &crate::dev::BuildRun,
) {
    let mut manager = match plugin_manager.lock() {
        Ok(manager) => manager,
        Err(error) => {
            log::error!("Plugin manager mutex poisoned: {}", error);
            return;
        }
    };

    let reload_ids = match plugin_filter {
        Some(plugin_id) => vec![plugin_id.to_string()],
        None => stale_plugin_ids(build_run),
    };
    if reload_ids.is_empty() {
        log::info!("Developer reload: all linked plugins are up to date");
        drop(manager);
        config.config_changed(ConfigKind::Plugins);
        events.send_plugins_changed();
        return;
    }

    let mut failures = Vec::new();
    for plugin_id in &reload_ids {
        if let Err(error) = manager.reload_plugin(plugin_id) {
            failures.push((plugin_id.clone(), error));
        }
    }
    drop(manager);

    if let Some(plugin_id) = plugin_filter {
        if let Some((_, error)) = failures.iter().find(|(id, _)| id == plugin_id) {
            log::error!("Failed to reload plugin {}: {}", plugin_id, error);
        } else {
            log::info!("Plugin {} reloaded successfully", plugin_id);
        }
    } else if failures.is_empty() {
        log::info!("Plugins reloaded successfully");
    } else {
        let failed = failures
            .iter()
            .map(|(id, _)| id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        log::error!("Failed to reload plugins: {}", failed);
    }
    config.config_changed(ConfigKind::Plugins);
    events.send_plugins_changed();
}

fn stale_plugin_ids(build_run: &crate::dev::BuildRun) -> Vec<String> {
    build_run
        .plans
        .iter()
        .filter(|plan| plan.needs_rebuild)
        .map(|plan| plan.plugin_id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::PluginLoader;
    use std::collections::HashMap;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    fn build_run(plans: &[(&str, bool)]) -> crate::dev::BuildRun {
        crate::dev::BuildRun {
            plans: plans
                .iter()
                .map(|(id, needs_rebuild)| qol_dev_build::PluginBuildPlan {
                    plugin_id: id.to_string(),
                    path: std::path::PathBuf::new(),
                    has_cargo: true,
                    supports_platform: true,
                    needs_rebuild: *needs_rebuild,
                    current_fingerprint: None,
                    last_built_fingerprint: None,
                    reason: String::new(),
                })
                .collect(),
            results: Vec::new(),
            fingerprints: HashMap::new(),
        }
    }

    #[test]
    fn stale_plugin_ids_returns_only_plans_needing_rebuild() {
        let run = build_run(&[
            ("plugin-a", false),
            ("plugin-b", true),
            ("plugin-c", true),
            ("plugin-d", false),
        ]);

        assert_eq!(stale_plugin_ids(&run), vec!["plugin-b", "plugin-c"]);
    }

    #[cfg(unix)]
    mod unix_integration {
        use super::*;

        const TARGET_ID: &str = "plugin-reload-fresh-target";
        const OTHER_ID: &str = "plugin-reload-fresh-other";

        fn write_daemon_plugin(
            plugins_dir: &std::path::Path,
            plugin_id: &str,
            version: &str,
        ) -> std::path::PathBuf {
            let plugin_dir = plugins_dir.join(plugin_id);
            std::fs::create_dir_all(&plugin_dir).unwrap();
            let script = plugin_dir.join("daemon");
            std::fs::write(&script, "#!/bin/sh\nexec sleep 30\n").unwrap();
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
            std::fs::write(
                plugin_dir.join("plugin.toml"),
                format!(
                    r#"
[plugin]
id = "{plugin_id}"
name = "{plugin_id}"
description = ""
version = "{version}"

[menu]
label = "{plugin_id}"
items = []

[daemon]
enabled = true
command = "daemon"
"#,
                ),
            )
            .unwrap();
            plugin_dir
        }

        fn insert_loaded_plugin(
            manager: &mut crate::plugins::PluginManager,
            plugin_id: &str,
            path: &Path,
        ) {
            let plugin = PluginLoader::load_plugin_with_id(plugin_id, path).unwrap();
            manager.insert_plugin_for_test(plugin);
            manager.ensure_plugin_daemon_running(plugin_id).unwrap();
        }

        fn loaded_manager() -> (
            Arc<Mutex<crate::plugins::PluginManager>>,
            std::path::PathBuf,
            crate::paths::TestPathRootGuard,
        ) {
            let _env_lock = crate::test_support::env_lock().blocking_lock();
            let _runtime_cache_lock = crate::test_support::runtime_cache_lock().blocking_lock();
            let root = tempfile::TempDir::new().unwrap();
            let path_guard = crate::paths::push_test_path_root(root.path());
            let plugins_dir = crate::paths::plugins_dir().unwrap();
            let target_dir = write_daemon_plugin(&plugins_dir, TARGET_ID, "1.0.0");
            let other_dir = write_daemon_plugin(&plugins_dir, OTHER_ID, "1.0.0");
            let config_dir = crate::paths::shared_config_dir().unwrap();
            crate::plugins::registry::record_release_install(&config_dir, TARGET_ID, target_dir)
                .unwrap();
            crate::plugins::registry::record_release_install(&config_dir, OTHER_ID, other_dir)
                .unwrap();
            let mut manager = crate::plugins::PluginManager::new();
            insert_loaded_plugin(&mut manager, TARGET_ID, &plugins_dir.join(TARGET_ID));
            insert_loaded_plugin(&mut manager, OTHER_ID, &plugins_dir.join(OTHER_ID));
            (Arc::new(Mutex::new(manager)), plugins_dir, path_guard)
        }

        fn reload(
            manager: &Arc<Mutex<crate::plugins::PluginManager>>,
            plugin_filter: Option<&str>,
            build_run: &crate::dev::BuildRun,
        ) {
            reload_plugins(
                Arc::clone(manager),
                Arc::new(crate::daemon::ConfigBus::new()),
                Arc::new(crate::daemon::EventBus::new()),
                plugin_filter,
                build_run,
            );
        }

        fn stop_all_daemons(manager: &Arc<Mutex<crate::plugins::PluginManager>>) {
            manager.lock().unwrap().shutdown();
        }

        #[test]
        fn fresh_plugins_keep_their_daemons_alive() {
            let (manager, _, _path_guard) = loaded_manager();
            let target_pid_before = manager
                .lock()
                .unwrap()
                .get(TARGET_ID)
                .unwrap()
                .daemon_pid()
                .unwrap();
            let other_pid_before = manager
                .lock()
                .unwrap()
                .get(OTHER_ID)
                .unwrap()
                .daemon_pid()
                .unwrap();

            reload(
                &manager,
                None,
                &build_run(&[(TARGET_ID, false), (OTHER_ID, false)]),
            );

            let guard = manager.lock().unwrap();
            assert_eq!(
                guard.get(TARGET_ID).unwrap().daemon_pid(),
                Some(target_pid_before),
                "an up-to-date plugin must not stop or respawn its daemon"
            );
            assert_eq!(
                guard.get(OTHER_ID).unwrap().daemon_pid(),
                Some(other_pid_before),
                "an up-to-date plugin must not stop or respawn its daemon"
            );
            drop(guard);
            stop_all_daemons(&manager);
        }

        #[test]
        fn changed_plugin_restarts_while_fresh_plugin_keeps_its_daemon() {
            let (manager, plugins_dir, _path_guard) = loaded_manager();
            let target_pid_before = manager
                .lock()
                .unwrap()
                .get(TARGET_ID)
                .unwrap()
                .daemon_pid()
                .unwrap();
            let other_pid_before = manager
                .lock()
                .unwrap()
                .get(OTHER_ID)
                .unwrap()
                .daemon_pid()
                .unwrap();

            write_daemon_plugin(&plugins_dir, TARGET_ID, "2.0.0");

            reload(
                &manager,
                None,
                &build_run(&[(TARGET_ID, true), (OTHER_ID, false)]),
            );

            let guard = manager.lock().unwrap();
            let target_pid_after = guard.get(TARGET_ID).unwrap().daemon_pid().unwrap();
            assert_eq!(
                guard.get(TARGET_ID).unwrap().manifest.plugin.version,
                "2.0.0"
            );
            assert_ne!(
                target_pid_after, target_pid_before,
                "a rebuilt plugin must restart with a fresh daemon"
            );
            assert!(!crate::process_utils::is_pid_alive(
                target_pid_before as i32
            ));
            assert_eq!(
                guard.get(OTHER_ID).unwrap().daemon_pid(),
                Some(other_pid_before),
                "an unrelated fresh plugin daemon must keep its exact process"
            );
            assert!(crate::process_utils::is_pid_alive(other_pid_before as i32));
            drop(guard);
            stop_all_daemons(&manager);
        }
    }
}
