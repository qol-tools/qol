#![cfg(feature = "dev")]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::daemon::DaemonEvent;
use crate::dev;

use super::dev_runtime::DevRuntimeService;
use super::restart::RestartPort;
use super::types::{
    AppState, MOCK_TARGET_PLUGIN_BUILD, MOCK_TARGET_SELF_RECOMPILE, MOCK_TARGET_SELF_UPDATE,
};

const RESTART_IDLE_POLL_MS: u64 = 250;

pub(super) fn queue_reload(state: &AppState) -> Result<(), &'static str> {
    let runtime = state.runtime.clone();
    if !runtime.try_start_build() {
        return Err("Build already in progress");
    }

    log::info!("Developer reload requested");

    let plugin_manager = state.plugin_manager.clone();
    let events = state.daemon.events.clone();
    let event_sink = runtime.create_core_event_sink(events.clone());
    let config_dir = crate::paths::shared_config_dir().ok();

    tokio::task::spawn_blocking(move || {
        struct BuildGuard {
            runtime: Arc<DevRuntimeService>,
        }
        impl Drop for BuildGuard {
            fn drop(&mut self) {
                self.runtime.finish_build();
            }
        }
        let _guard = BuildGuard { runtime };

        let dev_links = config_dir
            .as_deref()
            .map(dev::load_dev_links)
            .unwrap_or_default();
        let build_service = dev::default_build_application_service(event_sink.as_ref());
        let build_run = build_service.run(&dev_links, config_dir.as_deref());

        let all_succeeded =
            build_run.results.is_empty() || build_run.results.iter().all(|result| result.success);
        if !all_succeeded {
            return;
        }

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
    });

    Ok(())
}

pub(super) fn queue_self_recompile(state: &AppState) -> Result<(), &'static str> {
    if !state.runtime.try_start_self_recompile() {
        return Err("Self recompile already in progress");
    }

    log::info!("Developer self recompile requested");

    let events = state.daemon.events.clone();
    let plugin_manager = state.plugin_manager.clone();
    let runtime = state.runtime.clone();
    let restart = state.restart.clone();

    tokio::spawn(async move {
        struct RecompileGuard {
            runtime: Arc<DevRuntimeService>,
        }
        impl Drop for RecompileGuard {
            fn drop(&mut self) {
                self.runtime.finish_self_recompile();
            }
        }
        let _guard = RecompileGuard {
            runtime: runtime.clone(),
        };

        let progress_events = events.clone();
        let result = tokio::task::spawn_blocking(move || {
            dev::build_qol_tray_self_with_progress(|percent, phase| {
                progress_events.send(DaemonEvent::SelfRecompileProgress { percent, phase });
            })
        })
        .await;

        match result {
            Ok(build) if build.success => {
                events.send(DaemonEvent::SelfRecompileComplete);
                schedule_self_restart_after_idle(plugin_manager, runtime, restart);
            }
            Ok(build) => {
                let message = build_failure_message(&build.output);
                log::error!("Self recompile failed: {}", message);
                events.send(DaemonEvent::SelfRecompileFailed { message });
            }
            Err(error) => {
                let message = format!("Self recompile worker failed: {}", error);
                log::error!("{}", message);
                events.send(DaemonEvent::SelfRecompileFailed { message });
            }
        }
    });

    Ok(())
}

fn schedule_self_restart_after_idle(
    plugin_manager: Arc<Mutex<crate::plugins::PluginManager>>,
    runtime: Arc<DevRuntimeService>,
    restart: Arc<dyn RestartPort>,
) {
    if !runtime.try_mark_restart_pending() {
        return;
    }

    tokio::spawn(async move {
        loop {
            let idle = !runtime.build_in_progress()
                && !runtime.any_mock_target_running()
                && !runtime.self_recompile_in_progress();
            if idle {
                break;
            }
            tokio::time::sleep(Duration::from_millis(RESTART_IDLE_POLL_MS)).await;
        }

        let restart_binary = match restart.resolve_restart_binary() {
            Some(path) => path,
            None => {
                log::error!("Self recompile completed but restart binary could not be resolved");
                runtime.clear_restart_pending();
                return;
            }
        };

        if let Err(error) = restart.spawn_delayed_restart(&restart_binary) {
            log::error!(
                "Self recompile completed but restart spawn failed for {}: {}",
                restart_binary.display(),
                error
            );
            runtime.clear_restart_pending();
            return;
        }

        match plugin_manager.lock() {
            Ok(mut manager) => manager.shutdown(),
            Err(error) => {
                log::error!(
                    "Plugin manager lock poisoned during self restart: {}",
                    error
                );
            }
        }

        std::process::exit(0);
    });
}

fn build_failure_message(output: &str) -> String {
    output
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| "Self recompile failed".to_string())
}

pub(super) fn start_mock_targets(state: &AppState) -> Result<Vec<&'static str>, &'static str> {
    let runtime = state.runtime.clone();
    if runtime.any_mock_target_running() {
        return Err("Mock target already in progress");
    }

    let events = state.daemon.events.clone();
    let config_dir = crate::paths::shared_config_dir().ok();
    let fallback_plugin_ids = fallback_plugin_ids(state);
    let mut started = Vec::new();

    if runtime.start_mock_self_update(events.clone()).is_ok() {
        started.push(MOCK_TARGET_SELF_UPDATE);
    }
    if runtime.start_mock_self_recompile(events.clone()).is_ok() {
        started.push(MOCK_TARGET_SELF_RECOMPILE);
    }
    if runtime
        .start_mock_plugin_build(events, config_dir, fallback_plugin_ids)
        .is_ok()
    {
        started.push(MOCK_TARGET_PLUGIN_BUILD);
    }

    if started.is_empty() {
        return Err("No mock targets were started");
    }

    Ok(started)
}

pub(super) fn stop_mock_targets(state: &AppState) -> Vec<&'static str> {
    let runtime = state.runtime.clone();
    let mut stopped = Vec::new();

    if runtime.stop_mock_self_update() {
        stopped.push(MOCK_TARGET_SELF_UPDATE);
    }
    if runtime.stop_mock_self_recompile() {
        stopped.push(MOCK_TARGET_SELF_RECOMPILE);
    }
    if runtime.stop_mock_plugin_build() {
        stopped.push(MOCK_TARGET_PLUGIN_BUILD);
    }

    stopped
}

fn fallback_plugin_ids(state: &AppState) -> Vec<String> {
    state
        .dev_state
        .discovery
        .read()
        .map(|discovery| {
            discovery
                .plugins
                .iter()
                .map(|plugin| plugin.id.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}
