#![cfg(feature = "dev")]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::daemon::DaemonEvent;
use crate::dev;
use crate::dev::BuildResult;

use super::super::dev_runtime::DevRuntimeService;
use super::super::restart::RestartPort;
use super::super::types::AppState;

const RESTART_IDLE_POLL_MS: u64 = 250;

pub(super) fn queue_self_recompile(state: &AppState) -> Result<(), &'static str> {
    if !state.runtime.try_start_self_recompile() {
        return Err("Self recompile already in progress");
    }

    log::info!("Developer self recompile requested");
    tokio::spawn(run_self_recompile(SelfRecompileTask::from_state(state)));
    Ok(())
}

struct SelfRecompileTask {
    events: Arc<crate::daemon::EventBus>,
    plugin_manager: Arc<Mutex<crate::plugins::PluginManager>>,
    runtime: Arc<DevRuntimeService>,
    restart: Arc<dyn RestartPort>,
}

impl SelfRecompileTask {
    fn from_state(state: &AppState) -> Self {
        Self {
            events: state.daemon.events.clone(),
            plugin_manager: state.plugin_manager.clone(),
            runtime: state.runtime.clone(),
            restart: state.restart.clone(),
        }
    }
}

struct RecompileGuard {
    runtime: Arc<DevRuntimeService>,
}

impl Drop for RecompileGuard {
    fn drop(&mut self) {
        self.runtime.finish_self_recompile();
    }
}

async fn run_self_recompile(task: SelfRecompileTask) {
    let _guard = RecompileGuard {
        runtime: task.runtime.clone(),
    };
    let result = spawn_self_recompile(task.events.clone()).await;
    handle_recompile_result(task, result);
}

async fn spawn_self_recompile(
    events: Arc<crate::daemon::EventBus>,
) -> Result<BuildResult, tokio::task::JoinError> {
    tokio::task::spawn_blocking(move || {
        dev::build_qol_tray_self_with_progress(|percent, phase| {
            events.send(DaemonEvent::SelfRecompileProgress { percent, phase });
        })
    })
    .await
}

fn handle_recompile_result(
    task: SelfRecompileTask,
    result: Result<BuildResult, tokio::task::JoinError>,
) {
    match result {
        Ok(build) if build.success => handle_recompile_success(task),
        Ok(build) => handle_recompile_failure(&task.events, build_failure_message(&build.output)),
        Err(error) => handle_recompile_failure(
            &task.events,
            format!("Self recompile worker failed: {}", error),
        ),
    }
}

fn handle_recompile_success(task: SelfRecompileTask) {
    task.events.send(DaemonEvent::SelfRecompileComplete);
    schedule_self_restart_after_idle(task.plugin_manager, task.runtime, task.restart);
}

fn handle_recompile_failure(events: &crate::daemon::EventBus, message: String) {
    log::error!("Self recompile failed: {}", message);
    events.send(DaemonEvent::SelfRecompileFailed { message });
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
        wait_for_restart_idle(runtime.as_ref()).await;
        let Some(restart_binary) = resolve_restart_binary(runtime.as_ref(), restart.as_ref())
        else {
            return;
        };
        if !spawn_restart(runtime.as_ref(), restart.as_ref(), &restart_binary) {
            return;
        }
        shutdown_and_exit(plugin_manager);
    });
}

async fn wait_for_restart_idle(runtime: &DevRuntimeService) {
    loop {
        if restart_idle(runtime) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(RESTART_IDLE_POLL_MS)).await;
    }
}

fn restart_idle(runtime: &DevRuntimeService) -> bool {
    !runtime.build_in_progress()
        && !runtime.any_mock_target_running()
        && !runtime.self_recompile_in_progress()
}

fn resolve_restart_binary(
    runtime: &DevRuntimeService,
    restart: &dyn RestartPort,
) -> Option<std::path::PathBuf> {
    let Some(path) = restart.resolve_restart_binary() else {
        log::error!("Self recompile completed but restart binary could not be resolved");
        runtime.clear_restart_pending();
        return None;
    };
    Some(path)
}

fn spawn_restart(
    runtime: &DevRuntimeService,
    restart: &dyn RestartPort,
    restart_binary: &std::path::Path,
) -> bool {
    if let Err(error) = restart.spawn_delayed_restart(restart_binary) {
        log::error!(
            "Self recompile completed but restart spawn failed for {}: {}",
            restart_binary.display(),
            error
        );
        runtime.clear_restart_pending();
        return false;
    }
    true
}

fn shutdown_and_exit(plugin_manager: Arc<Mutex<crate::plugins::PluginManager>>) {
    match plugin_manager.lock() {
        Ok(mut manager) => manager.shutdown(),
        Err(error) => log::error!(
            "Plugin manager lock poisoned during self restart: {}",
            error
        ),
    }
    std::process::exit(0);
}

fn build_failure_message(output: &str) -> String {
    output
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| "Self recompile failed".to_string())
}
