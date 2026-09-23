use crate::dev::core::BuildStatus;
use crate::dev::state::BuildResultInfo;
use std::collections::HashMap;

pub use qol_dev_build::adapters::{BuildFingerprintStore, CargoPluginBuilder, CoreEventSink};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BuildStateProgress {
    pub status: BuildStatus,
    pub percent: u8,
    pub phase: String,
}

pub trait BuildStateStore: Send + Sync {
    fn mark_started(&self);
    fn update_plugin(&self, plugin_id: &str, status: BuildStatus, percent: u8, phase: &str);
    fn mark_finished(&self);
    fn store_results(&self, results: Vec<BuildResultInfo>);
    fn last_results(&self) -> Option<Vec<BuildResultInfo>>;
    fn is_building(&self) -> bool;
    fn snapshot(&self) -> HashMap<String, BuildStateProgress>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DevMockTarget {
    SelfUpdate,
    SelfRecompile,
    PluginBuild,
}

pub trait DevRuntimeStateStore: BuildStateStore + Send + Sync {
    fn try_start_build(&self) -> bool;
    fn finish_build(&self);
    fn build_in_progress(&self) -> bool;
    fn try_start_self_recompile(&self) -> bool;
    fn finish_self_recompile(&self);
    fn self_recompile_in_progress(&self) -> bool;
    fn try_mark_restart_pending(&self) -> bool;
    fn clear_restart_pending(&self);
    fn try_start_mock_target(&self, target: DevMockTarget) -> bool;
    fn request_stop_mock_target(&self, target: DevMockTarget) -> bool;
    fn mock_target_running(&self, target: DevMockTarget) -> bool;
    fn mock_target_cancelled(&self, target: DevMockTarget) -> bool;
    fn clear_mock_target(&self, target: DevMockTarget);
}
