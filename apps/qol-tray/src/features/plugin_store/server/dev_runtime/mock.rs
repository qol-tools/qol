#![cfg(feature = "dev")]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::daemon::{DaemonEvent, EventBus};
use crate::dev;
use crate::dev::adapters::traits::{CoreEventSink, DevMockTarget, DevRuntimeStateStore};
use crate::dev::core::{BuildStatus, CoreBuildResult, CoreEvent};

use super::core_events;

pub(super) fn start_mock_self_update(
    state: Arc<dyn DevRuntimeStateStore>,
    events: Arc<EventBus>,
) -> Result<(), &'static str> {
    if !state.try_start_mock_target(DevMockTarget::SelfUpdate) {
        return Err("Mock self-update already in progress");
    }

    tokio::spawn(run_mock_self_update(state, events));
    Ok(())
}

pub(super) fn stop_mock_self_update(state: &dyn DevRuntimeStateStore) -> bool {
    state.request_stop_mock_target(DevMockTarget::SelfUpdate)
}

pub(super) fn start_mock_self_recompile(
    state: Arc<dyn DevRuntimeStateStore>,
    events: Arc<EventBus>,
) -> Result<(), &'static str> {
    if !state.try_start_mock_target(DevMockTarget::SelfRecompile) {
        return Err("Mock self-recompile already in progress");
    }

    tokio::spawn(run_mock_self_recompile(state, events));
    Ok(())
}

pub(super) fn stop_mock_self_recompile(state: &dyn DevRuntimeStateStore) -> bool {
    state.request_stop_mock_target(DevMockTarget::SelfRecompile)
}

pub(super) fn start_mock_plugin_build(
    state: Arc<dyn DevRuntimeStateStore>,
    events: Arc<EventBus>,
    config_dir: Option<PathBuf>,
    fallback_plugin_ids: Vec<String>,
) -> Result<(), &'static str> {
    if !state.try_start_mock_target(DevMockTarget::PluginBuild) {
        return Err("Mock plugin build already in progress");
    }

    tokio::spawn(run_mock_plugin_build(
        state,
        events,
        config_dir,
        fallback_plugin_ids,
    ));
    Ok(())
}

pub(super) fn stop_mock_plugin_build(state: &dyn DevRuntimeStateStore) -> bool {
    state.request_stop_mock_target(DevMockTarget::PluginBuild)
}

struct MockTargetGuard {
    state: Arc<dyn DevRuntimeStateStore>,
    target: DevMockTarget,
}

impl MockTargetGuard {
    fn new(state: Arc<dyn DevRuntimeStateStore>, target: DevMockTarget) -> Self {
        Self { state, target }
    }
}

impl Drop for MockTargetGuard {
    fn drop(&mut self) {
        self.state.clear_mock_target(self.target);
    }
}

async fn run_mock_self_update(state: Arc<dyn DevRuntimeStateStore>, events: Arc<EventBus>) {
    let _guard = MockTargetGuard::new(state.clone(), DevMockTarget::SelfUpdate);
    for i in 0..=100u8 {
        if state.mock_target_cancelled(DevMockTarget::SelfUpdate) {
            break;
        }
        events.send(DaemonEvent::UpdateProgress { percent: i });
        tokio::time::sleep(tokio::time::Duration::from_millis(45)).await;
    }
    events.send(DaemonEvent::UpdateComplete);
}

async fn run_mock_self_recompile(state: Arc<dyn DevRuntimeStateStore>, events: Arc<EventBus>) {
    let _guard = MockTargetGuard::new(state.clone(), DevMockTarget::SelfRecompile);
    for i in 0..=100u8 {
        if state.mock_target_cancelled(DevMockTarget::SelfRecompile) {
            break;
        }
        events.send(DaemonEvent::SelfRecompileProgress {
            percent: i,
            phase: mock_recompile_phase(i).to_string(),
        });
        tokio::time::sleep(tokio::time::Duration::from_millis(45)).await;
    }
    events.send(DaemonEvent::SelfRecompileComplete);
}

async fn run_mock_plugin_build(
    state: Arc<dyn DevRuntimeStateStore>,
    events: Arc<EventBus>,
    config_dir: Option<PathBuf>,
    fallback_plugin_ids: Vec<String>,
) {
    let _guard = MockTargetGuard::new(state.clone(), DevMockTarget::PluginBuild);
    let event_sink = core_events::new_runtime_core_event_sink(events, state.clone());
    let plugin_ids = mock_plugin_ids(config_dir.as_deref(), fallback_plugin_ids);

    if state.mock_target_cancelled(DevMockTarget::PluginBuild) {
        publish_mock_build_complete(event_sink.as_ref(), Vec::new());
        return;
    }

    event_sink.publish(CoreEvent::BuildStarted);
    if plugin_ids.is_empty() {
        publish_mock_build_complete(event_sink.as_ref(), Vec::new());
        return;
    }

    queue_mock_plugins(&plugin_ids, event_sink.as_ref());
    for plugin_id in &plugin_ids {
        if !run_single_mock_plugin(plugin_id, state.as_ref(), event_sink.as_ref()).await {
            publish_mock_build_complete(event_sink.as_ref(), Vec::new());
            return;
        }
    }

    publish_mock_build_complete(event_sink.as_ref(), successful_mock_results(plugin_ids));
}

fn queue_mock_plugins(plugin_ids: &[String], event_sink: &dyn CoreEventSink) {
    for plugin_id in plugin_ids {
        event_sink.publish(CoreEvent::BuildPluginProgress {
            plugin_id: plugin_id.clone(),
            status: BuildStatus::Queued,
            percent: 0,
            phase: "Queued".to_string(),
        });
    }
}

async fn run_single_mock_plugin(
    plugin_id: &str,
    state: &dyn DevRuntimeStateStore,
    event_sink: &dyn CoreEventSink,
) -> bool {
    if state.mock_target_cancelled(DevMockTarget::PluginBuild) {
        return false;
    }

    publish_mock_plugin_start(plugin_id, event_sink);
    tokio::time::sleep(tokio::time::Duration::from_millis(120)).await;

    for done in 1..=24 {
        if state.mock_target_cancelled(DevMockTarget::PluginBuild) {
            return false;
        }
        publish_mock_plugin_progress(plugin_id, done, event_sink);
        tokio::time::sleep(tokio::time::Duration::from_millis(55)).await;
    }

    true
}

fn publish_mock_plugin_start(plugin_id: &str, event_sink: &dyn CoreEventSink) {
    event_sink.publish(CoreEvent::BuildPluginProgress {
        plugin_id: plugin_id.to_string(),
        status: BuildStatus::Building,
        percent: 0,
        phase: "0/24 preparing".to_string(),
    });
}

fn publish_mock_plugin_progress(plugin_id: &str, done: u8, event_sink: &dyn CoreEventSink) {
    let percent = ((done as usize * 100) / 24) as u8;
    let phase = format!("{done}/24 compiling");
    event_sink.publish(CoreEvent::BuildPluginProgress {
        plugin_id: plugin_id.to_string(),
        status: BuildStatus::Building,
        percent,
        phase,
    });
}

fn successful_mock_results(plugin_ids: Vec<String>) -> Vec<CoreBuildResult> {
    plugin_ids
        .into_iter()
        .map(|plugin_id| CoreBuildResult {
            plugin_id,
            success: true,
            output: "Mock build completed".to_string(),
            skipped: false,
        })
        .collect()
}

fn mock_recompile_phase(percent: u8) -> &'static str {
    match percent {
        0..=10 => "Preparing build",
        11..=35 => "Resolving dependencies",
        36..=95 => "Compiling crates",
        _ => "Finalizing build",
    }
}

fn mock_plugin_ids(config_dir: Option<&Path>, fallback_plugin_ids: Vec<String>) -> Vec<String> {
    let mut plugin_ids: Vec<String> = config_dir
        .map(dev::load_dev_links)
        .unwrap_or_default()
        .into_keys()
        .collect();
    if plugin_ids.is_empty() {
        plugin_ids = fallback_plugin_ids;
    }
    plugin_ids.sort();
    plugin_ids.dedup();
    plugin_ids
}

fn publish_mock_build_complete(event_sink: &dyn CoreEventSink, results: Vec<CoreBuildResult>) {
    event_sink.publish(CoreEvent::BuildComplete { results });
}
