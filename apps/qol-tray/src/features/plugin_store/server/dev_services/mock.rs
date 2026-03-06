#![cfg(feature = "dev")]

use super::super::dev_runtime::DevRuntimeService;
use super::super::helpers::shared_config_dir;
use super::super::types::{
    AppState, MOCK_TARGET_PLUGIN_BUILD, MOCK_TARGET_SELF_RECOMPILE, MOCK_TARGET_SELF_UPDATE,
};

pub(super) fn start_mock_targets(state: &AppState) -> Result<Vec<&'static str>, &'static str> {
    let runtime = state.runtime.clone();
    if runtime.any_mock_target_running() {
        return Err("Mock target already in progress");
    }

    let events = state.daemon.events.clone();
    let config_dir = shared_config_dir().ok();
    let fallback_plugin_ids = fallback_plugin_ids(state);
    let mut started = Vec::new();

    start_mock_self_update(runtime.as_ref(), events.clone(), &mut started);
    start_mock_self_recompile(runtime.as_ref(), events.clone(), &mut started);
    start_mock_plugin_build(
        runtime.as_ref(),
        events,
        config_dir,
        fallback_plugin_ids,
        &mut started,
    );

    if started.is_empty() {
        return Err("No mock targets were started");
    }
    Ok(started)
}

pub(super) fn queue_mock_plugin_build(state: &AppState) -> Result<(), &'static str> {
    let events = state.daemon.events.clone();
    let config_dir = shared_config_dir().ok();
    state
        .runtime
        .start_mock_plugin_build(events, config_dir, fallback_plugin_ids(state))
}

pub(super) fn stop_mock_targets(state: &AppState) -> Vec<&'static str> {
    let runtime = state.runtime.clone();
    let mut stopped = Vec::new();

    push_stopped(
        runtime.stop_mock_self_update(),
        MOCK_TARGET_SELF_UPDATE,
        &mut stopped,
    );
    push_stopped(
        runtime.stop_mock_self_recompile(),
        MOCK_TARGET_SELF_RECOMPILE,
        &mut stopped,
    );
    push_stopped(
        runtime.stop_mock_plugin_build(),
        MOCK_TARGET_PLUGIN_BUILD,
        &mut stopped,
    );
    stopped
}

fn start_mock_self_update(
    runtime: &DevRuntimeService,
    events: std::sync::Arc<crate::daemon::EventBus>,
    started: &mut Vec<&'static str>,
) {
    if runtime.start_mock_self_update(events).is_ok() {
        started.push(MOCK_TARGET_SELF_UPDATE);
    }
}

fn start_mock_self_recompile(
    runtime: &DevRuntimeService,
    events: std::sync::Arc<crate::daemon::EventBus>,
    started: &mut Vec<&'static str>,
) {
    if runtime.start_mock_self_recompile(events).is_ok() {
        started.push(MOCK_TARGET_SELF_RECOMPILE);
    }
}

fn start_mock_plugin_build(
    runtime: &DevRuntimeService,
    events: std::sync::Arc<crate::daemon::EventBus>,
    config_dir: Option<std::path::PathBuf>,
    fallback_plugin_ids: Vec<String>,
    started: &mut Vec<&'static str>,
) {
    if runtime
        .start_mock_plugin_build(events, config_dir, fallback_plugin_ids)
        .is_ok()
    {
        started.push(MOCK_TARGET_PLUGIN_BUILD);
    }
}

fn push_stopped(stopped_now: bool, target: &'static str, stopped: &mut Vec<&'static str>) {
    if stopped_now {
        stopped.push(target);
    }
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
