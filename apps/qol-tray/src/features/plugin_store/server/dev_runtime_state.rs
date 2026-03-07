use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::dev::adapters::{
    BuildStateProgress, BuildStateStore, DevMockTarget, DevRuntimeStateStore,
};
use crate::dev::core::BuildStatus;
use crate::dev::state::BuildResultInfo;

#[derive(Debug, Default)]
struct MockTargetState {
    in_progress: AtomicBool,
    cancel: AtomicBool,
}

#[derive(Debug, Default)]
struct RuntimeBuildState {
    building: bool,
    progress: HashMap<String, BuildStateProgress>,
    last_results: Option<Vec<BuildResultInfo>>,
}

#[derive(Default)]
pub(super) struct InMemoryDevRuntimeState {
    build_in_progress: AtomicBool,
    build_state: Mutex<RuntimeBuildState>,
    self_recompile_in_progress: AtomicBool,
    self_restart_pending: AtomicBool,
    mock_self_update: MockTargetState,
    mock_self_recompile: MockTargetState,
    mock_plugin_build: MockTargetState,
}

impl InMemoryDevRuntimeState {
    fn target_state(&self, target: DevMockTarget) -> &MockTargetState {
        match target {
            DevMockTarget::SelfUpdate => &self.mock_self_update,
            DevMockTarget::SelfRecompile => &self.mock_self_recompile,
            DevMockTarget::PluginBuild => &self.mock_plugin_build,
        }
    }

    fn with_build_state<T>(&self, default: T, op: impl FnOnce(&mut RuntimeBuildState) -> T) -> T {
        match self.build_state.lock() {
            Ok(mut guard) => op(&mut guard),
            Err(error) => {
                log::error!("Build state lock poisoned: {}", error);
                default
            }
        }
    }
}

impl BuildStateStore for InMemoryDevRuntimeState {
    fn mark_started(&self) {
        self.with_build_state((), |s| {
            s.building = true;
            s.progress.clear();
            s.last_results = None;
        });
    }

    fn update_plugin(&self, plugin_id: &str, status: BuildStatus, percent: u8, phase: &str) {
        self.with_build_state((), |s| {
            s.building = true;
            s.progress.insert(
                plugin_id.to_string(),
                BuildStateProgress {
                    status,
                    percent,
                    phase: phase.to_string(),
                },
            );
        });
    }

    fn mark_finished(&self) {
        self.with_build_state((), |s| {
            s.building = false;
            s.progress.clear();
        });
    }

    fn store_results(&self, results: Vec<BuildResultInfo>) {
        self.with_build_state((), |s| {
            s.last_results = Some(results);
        });
    }

    fn last_results(&self) -> Option<Vec<BuildResultInfo>> {
        self.with_build_state(None, |s| s.last_results.clone())
    }

    fn is_building(&self) -> bool {
        self.with_build_state(false, |s| s.building)
    }

    fn snapshot(&self) -> HashMap<String, BuildStateProgress> {
        self.with_build_state(HashMap::new(), |s| s.progress.clone())
    }
}

impl DevRuntimeStateStore for InMemoryDevRuntimeState {
    fn try_start_build(&self) -> bool {
        !self.build_in_progress.swap(true, Ordering::SeqCst)
    }

    fn finish_build(&self) {
        self.build_in_progress.store(false, Ordering::SeqCst);
        self.mark_finished();
    }

    fn build_in_progress(&self) -> bool {
        self.build_in_progress.load(Ordering::SeqCst)
    }

    fn try_start_self_recompile(&self) -> bool {
        !self.self_recompile_in_progress.swap(true, Ordering::SeqCst)
    }

    fn finish_self_recompile(&self) {
        self.self_recompile_in_progress
            .store(false, Ordering::SeqCst);
    }

    fn self_recompile_in_progress(&self) -> bool {
        self.self_recompile_in_progress.load(Ordering::SeqCst)
    }

    fn try_mark_restart_pending(&self) -> bool {
        !self.self_restart_pending.swap(true, Ordering::SeqCst)
    }

    fn clear_restart_pending(&self) {
        self.self_restart_pending.store(false, Ordering::SeqCst);
    }

    fn try_start_mock_target(&self, target: DevMockTarget) -> bool {
        let state = self.target_state(target);
        if state.in_progress.swap(true, Ordering::SeqCst) {
            return false;
        }
        state.cancel.store(false, Ordering::SeqCst);
        true
    }

    fn request_stop_mock_target(&self, target: DevMockTarget) -> bool {
        let state = self.target_state(target);
        if !state.in_progress.load(Ordering::SeqCst) {
            return false;
        }
        state.cancel.store(true, Ordering::SeqCst);
        true
    }

    fn mock_target_running(&self, target: DevMockTarget) -> bool {
        self.target_state(target).in_progress.load(Ordering::SeqCst)
    }

    fn mock_target_cancelled(&self, target: DevMockTarget) -> bool {
        self.target_state(target).cancel.load(Ordering::SeqCst)
    }

    fn clear_mock_target(&self, target: DevMockTarget) {
        let state = self.target_state(target);
        state.in_progress.store(false, Ordering::SeqCst);
        state.cancel.store(false, Ordering::SeqCst);
    }
}

pub(super) fn in_memory_runtime_state() -> Arc<dyn DevRuntimeStateStore> {
    Arc::new(InMemoryDevRuntimeState::default())
}
