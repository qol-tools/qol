#![cfg(feature = "dev")]

use std::sync::Arc;

use crate::daemon::{DaemonEvent, EventBus};
use crate::dev::adapters::traits::{CoreEventSink, DevRuntimeStateStore};
use crate::dev::core::{BuildStatus, CoreBuildResult, CoreEvent};
use crate::dev::state::BuildResultInfo;

struct RuntimeCoreEventSink {
    events: Arc<EventBus>,
    state: Arc<dyn DevRuntimeStateStore>,
}

impl CoreEventSink for RuntimeCoreEventSink {
    fn publish(&self, event: CoreEvent) {
        publish_core_event(self.events.as_ref(), self.state.as_ref(), event);
    }
}

pub(super) fn new_runtime_core_event_sink(
    events: Arc<EventBus>,
    state: Arc<dyn DevRuntimeStateStore>,
) -> Arc<dyn CoreEventSink> {
    Arc::new(RuntimeCoreEventSink { events, state })
}

pub(super) fn publish_core_event(
    events: &EventBus,
    state_store: &dyn DevRuntimeStateStore,
    event: CoreEvent,
) {
    match event {
        CoreEvent::BuildStarted => publish_build_started(events, state_store),
        CoreEvent::BuildPluginProgress {
            plugin_id,
            status,
            percent,
            phase,
        } => publish_plugin_progress(events, state_store, plugin_id, status, percent, phase),
        CoreEvent::BuildComplete { results } => {
            publish_build_complete(events, state_store, results)
        }
    }
}

fn publish_build_started(events: &EventBus, state_store: &dyn DevRuntimeStateStore) {
    state_store.mark_started();
    events.send(DaemonEvent::BuildStarted);
}

fn publish_plugin_progress(
    events: &EventBus,
    state_store: &dyn DevRuntimeStateStore,
    plugin_id: String,
    status: BuildStatus,
    percent: u8,
    phase: String,
) {
    state_store.update_plugin(&plugin_id, status, percent, &phase);
    events.send(DaemonEvent::BuildPluginProgress {
        plugin_id,
        status: status.as_str().to_string(),
        percent,
        phase,
    });
}

fn publish_build_complete(
    events: &EventBus,
    state_store: &dyn DevRuntimeStateStore,
    results: Vec<CoreBuildResult>,
) {
    let mapped = map_core_results(results);
    state_store.mark_finished();
    state_store.store_results(mapped.clone());
    events.send(DaemonEvent::BuildComplete { results: mapped });
}

fn map_core_results(results: Vec<CoreBuildResult>) -> Vec<BuildResultInfo> {
    results
        .into_iter()
        .map(|result| BuildResultInfo {
            plugin_id: result.plugin_id,
            success: result.success,
            output: result.output,
            skipped: result.skipped,
        })
        .collect()
}
