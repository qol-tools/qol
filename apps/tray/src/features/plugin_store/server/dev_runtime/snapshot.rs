#![cfg(feature = "dev")]

use std::collections::HashMap;

use crate::dev::adapters::{DevMockTarget, DevRuntimeStateStore};
use crate::dev::state::BuildResultInfo;

use super::super::types::{
    BuildProgressSnapshot, BuildStateResponse, MockTargetInfo, MOCK_TARGET_PLUGIN_BUILD,
    MOCK_TARGET_SELF_RECOMPILE, MOCK_TARGET_SELF_UPDATE,
};

pub(super) fn build_state_snapshot(state: &dyn DevRuntimeStateStore) -> BuildStateResponse {
    let building = state.build_in_progress() || state.is_building();
    BuildStateResponse {
        building,
        progress: building_progress(state, building),
        results: build_results(state, building),
    }
}

pub(super) fn mock_target_infos(state: &dyn DevRuntimeStateStore) -> Vec<MockTargetInfo> {
    vec![
        mock_target_info(
            state,
            DevMockTarget::SelfUpdate,
            MOCK_TARGET_SELF_UPDATE,
            "Self Update",
        ),
        mock_target_info(
            state,
            DevMockTarget::SelfRecompile,
            MOCK_TARGET_SELF_RECOMPILE,
            "Self Recompile",
        ),
        mock_target_info(
            state,
            DevMockTarget::PluginBuild,
            MOCK_TARGET_PLUGIN_BUILD,
            "Plugin Build",
        ),
    ]
}

pub(super) fn any_mock_target_running(state: &dyn DevRuntimeStateStore) -> bool {
    state.mock_target_running(DevMockTarget::SelfUpdate)
        || state.mock_target_running(DevMockTarget::SelfRecompile)
        || state.mock_target_running(DevMockTarget::PluginBuild)
}

fn building_progress(
    state: &dyn DevRuntimeStateStore,
    building: bool,
) -> HashMap<String, BuildProgressSnapshot> {
    if !building {
        return HashMap::new();
    }

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
}

fn build_results(state: &dyn DevRuntimeStateStore, building: bool) -> Option<Vec<BuildResultInfo>> {
    if building {
        return None;
    }
    state.last_results()
}

fn mock_target_info(
    state: &dyn DevRuntimeStateStore,
    target: DevMockTarget,
    id: &'static str,
    label: &'static str,
) -> MockTargetInfo {
    MockTargetInfo {
        id,
        label,
        running: state.mock_target_running(target),
        supports_stop: true,
    }
}
