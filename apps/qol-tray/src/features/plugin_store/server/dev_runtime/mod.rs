#![cfg(feature = "dev")]

mod core_events;
mod mock;
mod snapshot;

use std::path::PathBuf;
use std::sync::Arc;

use crate::daemon::EventBus;
use crate::dev::adapters::{CoreEventSink, DevRuntimeStateStore};

use super::dev_runtime_state::in_memory_runtime_state;
use super::types::{BuildStateResponse, MockTargetInfo};

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
        core_events::new_runtime_core_event_sink(events, Arc::clone(&self.state))
    }

    pub(super) fn build_state_snapshot(&self) -> BuildStateResponse {
        snapshot::build_state_snapshot(self.state.as_ref())
    }

    pub(super) fn list_mock_targets(&self) -> Vec<MockTargetInfo> {
        snapshot::mock_target_infos(self.state.as_ref())
    }

    pub(super) fn any_mock_target_running(&self) -> bool {
        snapshot::any_mock_target_running(self.state.as_ref())
    }

    pub(super) fn start_mock_self_update(&self, events: Arc<EventBus>) -> Result<(), &'static str> {
        mock::start_mock_self_update(Arc::clone(&self.state), events)
    }

    pub(super) fn stop_mock_self_update(&self) -> bool {
        mock::stop_mock_self_update(self.state.as_ref())
    }

    pub(super) fn start_mock_self_recompile(
        &self,
        events: Arc<EventBus>,
    ) -> Result<(), &'static str> {
        mock::start_mock_self_recompile(Arc::clone(&self.state), events)
    }

    pub(super) fn stop_mock_self_recompile(&self) -> bool {
        mock::stop_mock_self_recompile(self.state.as_ref())
    }

    pub(super) fn start_mock_plugin_build(
        &self,
        events: Arc<EventBus>,
        config_dir: Option<PathBuf>,
        fallback_plugin_ids: Vec<String>,
    ) -> Result<(), &'static str> {
        mock::start_mock_plugin_build(
            Arc::clone(&self.state),
            events,
            config_dir,
            fallback_plugin_ids,
        )
    }

    pub(super) fn stop_mock_plugin_build(&self) -> bool {
        mock::stop_mock_plugin_build(self.state.as_ref())
    }
}

pub(super) fn new_dev_runtime() -> Arc<DevRuntimeService> {
    Arc::new(DevRuntimeService::new())
}
