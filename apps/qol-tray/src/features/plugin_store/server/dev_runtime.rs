#![cfg(feature = "dev")]

use std::collections::HashMap;
use std::sync::Arc;

use crate::daemon::{DaemonEvent, EventBus};
use crate::dev::adapters::traits::{CoreEventSink, DevMockTarget, DevRuntimeStateStore};
use crate::dev::core::{BuildStatus, CoreBuildResult, CoreEvent};
use crate::dev;
use crate::dev::state::BuildResultInfo;

use super::dev_runtime_state::in_memory_runtime_state;
use super::types::{
    BuildProgressSnapshot, BuildStateResponse, MockTargetInfo, MOCK_TARGET_PLUGIN_BUILD,
    MOCK_TARGET_SELF_RECOMPILE, MOCK_TARGET_SELF_UPDATE,
};

pub(super) struct DevRuntimeService {
    state: Arc<dyn DevRuntimeStateStore>,
}

impl DevRuntimeService {
    pub(super) fn new() -> Self {
        Self {
            state: in_memory_runtime_state(),
        }
    }

    pub(super) fn try_start_build(&self) -> bool {
        self.state.try_start_build()
    }

    pub(super) fn finish_build(&self) {
        self.state.finish_build();
    }

    pub(super) fn build_in_progress(&self) -> bool {
        self.state.build_in_progress()
    }

    pub(super) fn try_start_self_recompile(&self) -> bool {
        self.state.try_start_self_recompile()
    }

    pub(super) fn finish_self_recompile(&self) {
        self.state.finish_self_recompile();
    }

    pub(super) fn self_recompile_in_progress(&self) -> bool {
        self.state.self_recompile_in_progress()
    }

    pub(super) fn try_mark_restart_pending(&self) -> bool {
        self.state.try_mark_restart_pending()
    }

    pub(super) fn clear_restart_pending(&self) {
        self.state.clear_restart_pending();
    }

    pub(super) fn create_core_event_sink(&self, events: Arc<EventBus>) -> Arc<dyn CoreEventSink> {
        Arc::new(RuntimeCoreEventSink {
            events,
            state: Arc::clone(&self.state),
        })
    }

    pub(super) fn build_state_snapshot(&self) -> BuildStateResponse {
        read_build_state_snapshot(self.state.as_ref())
    }

    pub(super) fn list_mock_targets(&self) -> Vec<MockTargetInfo> {
        mock_target_infos(self.state.as_ref())
    }

    pub(super) fn any_mock_target_running(&self) -> bool {
        any_mock_target_running(self.state.as_ref())
    }

    pub(super) fn start_mock_self_update(&self, events: Arc<EventBus>) -> Result<(), &'static str> {
        if !self.state.try_start_mock_target(DevMockTarget::SelfUpdate) {
            return Err("Mock self-update already in progress");
        }
        let state = Arc::clone(&self.state);
        tokio::spawn(async move {
            let _guard = MockTargetGuard::new(Arc::clone(&state), DevMockTarget::SelfUpdate);

            for i in 0..=100u8 {
                if state.mock_target_cancelled(DevMockTarget::SelfUpdate) {
                    break;
                }
                events.send(DaemonEvent::UpdateProgress { percent: i });
                tokio::time::sleep(tokio::time::Duration::from_millis(45)).await;
            }

            events.send(DaemonEvent::UpdateComplete);
        });
        Ok(())
    }

    pub(super) fn stop_mock_self_update(&self) -> bool {
        self.state.request_stop_mock_target(DevMockTarget::SelfUpdate)
    }

    pub(super) fn start_mock_self_recompile(
        &self,
        events: Arc<EventBus>,
    ) -> Result<(), &'static str> {
        if !self.state.try_start_mock_target(DevMockTarget::SelfRecompile) {
            return Err("Mock self-recompile already in progress");
        }
        let state = Arc::clone(&self.state);
        tokio::spawn(async move {
            let _guard =
                MockTargetGuard::new(Arc::clone(&state), DevMockTarget::SelfRecompile);

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
        });
        Ok(())
    }

    pub(super) fn stop_mock_self_recompile(&self) -> bool {
        self.state
            .request_stop_mock_target(DevMockTarget::SelfRecompile)
    }

    pub(super) fn start_mock_plugin_build(
        &self,
        events: Arc<EventBus>,
        config_dir: Option<std::path::PathBuf>,
        fallback_plugin_ids: Vec<String>,
    ) -> Result<(), &'static str> {
        if !self.state.try_start_mock_target(DevMockTarget::PluginBuild) {
            return Err("Mock plugin build already in progress");
        }

        let state = Arc::clone(&self.state);
        tokio::spawn(async move {
            let _guard = MockTargetGuard::new(Arc::clone(&state), DevMockTarget::PluginBuild);

            let event_sink = RuntimeCoreEventSink {
                events,
                state: Arc::clone(&state),
            };
            let plugin_ids = mock_plugin_ids(config_dir.as_deref(), fallback_plugin_ids);

            if state.mock_target_cancelled(DevMockTarget::PluginBuild) {
                publish_mock_build_complete(&event_sink, Vec::new());
                return;
            }

            event_sink.publish(CoreEvent::BuildStarted);
            if plugin_ids.is_empty() {
                publish_mock_build_complete(&event_sink, Vec::new());
                return;
            }

            for plugin_id in &plugin_ids {
                event_sink.publish(CoreEvent::BuildPluginProgress {
                    plugin_id: plugin_id.clone(),
                    status: BuildStatus::Queued,
                    percent: 0,
                    phase: "Queued".to_string(),
                });
            }

            for plugin_id in &plugin_ids {
                if state.mock_target_cancelled(DevMockTarget::PluginBuild) {
                    publish_mock_build_complete(&event_sink, Vec::new());
                    return;
                }

                event_sink.publish(CoreEvent::BuildPluginProgress {
                    plugin_id: plugin_id.clone(),
                    status: BuildStatus::Building,
                    percent: 0,
                    phase: "0/24 preparing".to_string(),
                });
                tokio::time::sleep(tokio::time::Duration::from_millis(120)).await;

                for done in 1..=24 {
                    if state.mock_target_cancelled(DevMockTarget::PluginBuild) {
                        publish_mock_build_complete(&event_sink, Vec::new());
                        return;
                    }

                    let percent = ((done * 100) / 24) as u8;
                    let phase = format!("{}/24 compiling", done);
                    event_sink.publish(CoreEvent::BuildPluginProgress {
                        plugin_id: plugin_id.clone(),
                        status: BuildStatus::Building,
                        percent,
                        phase,
                    });
                    tokio::time::sleep(tokio::time::Duration::from_millis(55)).await;
                }
            }

            let results = plugin_ids
                .into_iter()
                .map(|plugin_id| CoreBuildResult {
                    plugin_id,
                    success: true,
                    output: "Mock build completed".to_string(),
                    skipped: false,
                })
                .collect();
            publish_mock_build_complete(&event_sink, results);
        });

        Ok(())
    }

    pub(super) fn stop_mock_plugin_build(&self) -> bool {
        self.state.request_stop_mock_target(DevMockTarget::PluginBuild)
    }
}

pub(super) fn new_dev_runtime() -> Arc<DevRuntimeService> {
    Arc::new(DevRuntimeService::new())
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

struct RuntimeCoreEventSink {
    events: Arc<EventBus>,
    state: Arc<dyn DevRuntimeStateStore>,
}

impl CoreEventSink for RuntimeCoreEventSink {
    fn publish(&self, event: CoreEvent) {
        publish_core_event(self.events.as_ref(), self.state.as_ref(), event);
    }
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

pub(super) fn publish_core_event(
    events: &EventBus,
    state_store: &dyn DevRuntimeStateStore,
    event: CoreEvent,
) {
    match event {
        CoreEvent::BuildStarted => {
            state_store.mark_started();
            events.send(DaemonEvent::BuildStarted);
        }
        CoreEvent::BuildPluginProgress {
            plugin_id,
            status,
            percent,
            phase,
        } => {
            state_store.update_plugin(&plugin_id, status, percent, &phase);
            events.send(DaemonEvent::BuildPluginProgress {
                plugin_id,
                status: status.as_str().to_string(),
                percent,
                phase,
            });
        }
        CoreEvent::BuildComplete { results } => {
            let mapped = map_core_results(results);
            state_store.mark_finished();
            events.send(DaemonEvent::BuildComplete { results: mapped });
        }
    }
}

fn read_build_state_snapshot(state: &dyn DevRuntimeStateStore) -> BuildStateResponse {
    let atomic_building = state.build_in_progress();
    let building = atomic_building || state.is_building();
    let progress = if building {
        state
            .snapshot()
            .into_iter()
            .map(|(plugin_id, progress)| {
                (
                    plugin_id,
                    BuildProgressSnapshot {
                        status: progress.status.as_str().to_string(),
                        percent: progress.percent,
                        phase: progress.phase,
                    },
                )
            })
            .collect()
    } else {
        HashMap::new()
    };
    BuildStateResponse { building, progress }
}

fn mock_target_infos(state: &dyn DevRuntimeStateStore) -> Vec<MockTargetInfo> {
    vec![
        MockTargetInfo {
            id: MOCK_TARGET_SELF_UPDATE,
            label: "Self Update",
            running: state.mock_target_running(DevMockTarget::SelfUpdate),
            supports_stop: true,
        },
        MockTargetInfo {
            id: MOCK_TARGET_SELF_RECOMPILE,
            label: "Self Recompile",
            running: state.mock_target_running(DevMockTarget::SelfRecompile),
            supports_stop: true,
        },
        MockTargetInfo {
            id: MOCK_TARGET_PLUGIN_BUILD,
            label: "Plugin Build",
            running: state.mock_target_running(DevMockTarget::PluginBuild),
            supports_stop: true,
        },
    ]
}

fn any_mock_target_running(state: &dyn DevRuntimeStateStore) -> bool {
    state.mock_target_running(DevMockTarget::SelfUpdate)
        || state.mock_target_running(DevMockTarget::SelfRecompile)
        || state.mock_target_running(DevMockTarget::PluginBuild)
}

fn mock_recompile_phase(percent: u8) -> &'static str {
    match percent {
        0..=10 => "Preparing build",
        11..=35 => "Resolving dependencies",
        36..=95 => "Compiling crates",
        _ => "Finalizing build",
    }
}

fn mock_plugin_ids(
    config_dir: Option<&std::path::Path>,
    fallback_plugin_ids: Vec<String>,
) -> Vec<String> {
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
