use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::daemon::EventBus;
use crate::dev;
use crate::dev::adapters::{CoreEventSink, DevMockTarget, DevRuntimeStateStore};
use crate::dev::core::{BuildStatus, CoreBuildResult, CoreEvent};

const PREPARE_DELAY: Duration = Duration::from_millis(120);
const COMPILE_DELAY: Duration = Duration::from_millis(55);

pub(super) async fn run(
    state: Arc<dyn DevRuntimeStateStore>,
    events: Arc<EventBus>,
    config_dir: Option<PathBuf>,
    fallback_plugin_ids: Vec<String>,
) {
    let _guard = super::enter_mock_target(Arc::clone(&state), DevMockTarget::PluginBuild);
    let event_sink = super::new_mock_core_event_sink(events, Arc::clone(&state));
    let plugin_ids = mock_plugin_ids(config_dir.as_deref(), fallback_plugin_ids);
    run_build(plugin_ids, state.as_ref(), event_sink.as_ref()).await;
}

async fn run_build(
    plugin_ids: Vec<String>,
    state: &dyn DevRuntimeStateStore,
    event_sink: &dyn CoreEventSink,
) {
    if super::mock_target_cancelled(state, DevMockTarget::PluginBuild) {
        super::publish_mock_build_complete(event_sink, Vec::new());
        return;
    }
    event_sink.publish(CoreEvent::BuildStarted);
    if plugin_ids.is_empty() {
        super::publish_mock_build_complete(event_sink, Vec::new());
        return;
    }
    run_build_loop(plugin_ids, state, event_sink).await;
}

async fn run_build_loop(
    plugin_ids: Vec<String>,
    state: &dyn DevRuntimeStateStore,
    event_sink: &dyn CoreEventSink,
) {
    queue_plugins(&plugin_ids, event_sink);
    for plugin_id in &plugin_ids {
        if !run_single_plugin(plugin_id, state, event_sink).await {
            super::publish_mock_build_complete(event_sink, Vec::new());
            return;
        }
    }
    super::publish_mock_build_complete(event_sink, successful_results(plugin_ids));
}

fn queue_plugins(plugin_ids: &[String], event_sink: &dyn CoreEventSink) {
    for plugin_id in plugin_ids {
        event_sink.publish(CoreEvent::BuildPluginProgress {
            plugin_id: plugin_id.clone(),
            status: BuildStatus::Queued,
            percent: 0,
            phase: "Queued".to_string(),
        });
    }
}

async fn run_single_plugin(
    plugin_id: &str,
    state: &dyn DevRuntimeStateStore,
    event_sink: &dyn CoreEventSink,
) -> bool {
    if super::mock_target_cancelled(state, DevMockTarget::PluginBuild) {
        return false;
    }

    publish_plugin_start(plugin_id, event_sink);
    tokio::time::sleep(PREPARE_DELAY).await;

    for done in 1..=24 {
        if super::mock_target_cancelled(state, DevMockTarget::PluginBuild) {
            return false;
        }

        publish_plugin_progress(plugin_id, done, event_sink);
        tokio::time::sleep(COMPILE_DELAY).await;
    }

    true
}

fn publish_plugin_start(plugin_id: &str, event_sink: &dyn CoreEventSink) {
    event_sink.publish(CoreEvent::BuildPluginProgress {
        plugin_id: plugin_id.to_string(),
        status: BuildStatus::Building,
        percent: 0,
        phase: "0/24 preparing".to_string(),
    });
}

fn publish_plugin_progress(plugin_id: &str, done: u8, event_sink: &dyn CoreEventSink) {
    let percent = ((done as usize * 100) / 24) as u8;
    let phase = format!("{done}/24 compiling");
    event_sink.publish(CoreEvent::BuildPluginProgress {
        plugin_id: plugin_id.to_string(),
        status: BuildStatus::Building,
        percent,
        phase,
    });
}

fn successful_results(plugin_ids: Vec<String>) -> Vec<CoreBuildResult> {
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
