use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinSet;

use crate::daemon::EventBus;
use crate::dev;
use crate::dev::adapters::{CoreEventSink, DevMockTarget, DevRuntimeStateStore};
use crate::dev::core::{BuildStatus, CoreBuildResult, CoreEvent};

const STEP_COUNT: u64 = 24;
const MIN_BUILD_MS: u64 = 1000;
const MAX_BUILD_MS: u64 = 3000;

pub(super) async fn run(
    state: Arc<dyn DevRuntimeStateStore>,
    events: Arc<EventBus>,
    config_dir: Option<PathBuf>,
    fallback_plugin_ids: Vec<String>,
) {
    let _guard = super::enter_mock_target(Arc::clone(&state), DevMockTarget::PluginBuild);
    let event_sink = super::new_mock_core_event_sink(events, Arc::clone(&state));
    let plugin_ids = mock_plugin_ids(config_dir.as_deref(), fallback_plugin_ids);
    run_build(plugin_ids, Arc::clone(&state), event_sink).await;
}

async fn run_build(
    plugin_ids: Vec<String>,
    state: Arc<dyn DevRuntimeStateStore>,
    event_sink: Arc<dyn CoreEventSink>,
) {
    if super::mock_target_cancelled(state.as_ref(), DevMockTarget::PluginBuild) {
        super::publish_mock_build_complete(event_sink.as_ref(), Vec::new());
        return;
    }
    event_sink.publish(CoreEvent::BuildStarted);
    if plugin_ids.is_empty() {
        super::publish_mock_build_complete(event_sink.as_ref(), Vec::new());
        return;
    }
    run_build_loop(plugin_ids, state, event_sink).await;
}

async fn run_build_loop(
    plugin_ids: Vec<String>,
    state: Arc<dyn DevRuntimeStateStore>,
    event_sink: Arc<dyn CoreEventSink>,
) {
    queue_plugins(&plugin_ids, event_sink.as_ref());

    let mut tasks = JoinSet::new();
    for plugin_id in &plugin_ids {
        let plugin_id = plugin_id.clone();
        let state = Arc::clone(&state);
        let sink = Arc::clone(&event_sink);
        tasks.spawn(
            async move { run_single_plugin(&plugin_id, state.as_ref(), sink.as_ref()).await },
        );
    }

    let mut cancelled = false;
    while let Some(result) = tasks.join_next().await {
        if let Ok(false) = result {
            cancelled = true;
        }
    }

    if cancelled {
        super::publish_mock_build_complete(event_sink.as_ref(), Vec::new());
        return;
    }
    tokio::time::sleep(Duration::from_millis(80)).await;
    super::publish_mock_build_complete(event_sink.as_ref(), successful_results(plugin_ids));
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

    let (prepare_delay, compile_delay) = mock_build_delays(plugin_id);
    publish_plugin_start(plugin_id, event_sink);
    tokio::time::sleep(prepare_delay).await;

    for done in 1..=STEP_COUNT as u8 {
        if super::mock_target_cancelled(state, DevMockTarget::PluginBuild) {
            return false;
        }

        publish_plugin_progress(plugin_id, done, event_sink);
        tokio::time::sleep(compile_delay).await;
    }

    publish_plugin_success(plugin_id, event_sink);
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
    let percent = ((done as u64 * 100) / STEP_COUNT) as u8;
    let phase = format!("{done}/{STEP_COUNT} compiling");
    event_sink.publish(CoreEvent::BuildPluginProgress {
        plugin_id: plugin_id.to_string(),
        status: BuildStatus::Building,
        percent,
        phase,
    });
}

fn publish_plugin_success(plugin_id: &str, event_sink: &dyn CoreEventSink) {
    event_sink.publish(CoreEvent::BuildPluginProgress {
        plugin_id: plugin_id.to_string(),
        status: BuildStatus::Success,
        percent: 100,
        phase: "Build complete".to_string(),
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

fn mock_build_delays(plugin_id: &str) -> (Duration, Duration) {
    let hash: u64 = plugin_id
        .bytes()
        .fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as u64;
    let total_ms = MIN_BUILD_MS + (hash.wrapping_add(nanos) % (MAX_BUILD_MS - MIN_BUILD_MS + 1));
    let prepare_ms = total_ms / 12;
    let compile_ms = (total_ms - prepare_ms) / STEP_COUNT;
    (
        Duration::from_millis(prepare_ms),
        Duration::from_millis(compile_ms),
    )
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
