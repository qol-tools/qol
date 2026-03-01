use crate::dev::core::{BuildStatus, CoreEvent};
use crate::dev::state::BuildResultInfo;
use crate::dev::BuildResult;
use std::collections::HashMap;
use std::path::Path;

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

pub trait CoreEventSink: Send + Sync {
    fn publish(&self, event: CoreEvent);
}

pub trait CargoPluginBuilder: Send + Sync {
    fn build_plugin_with_progress(
        &self,
        plugin_id: &str,
        path: &Path,
        on_progress: &mut dyn FnMut(u8, String),
    ) -> BuildResult;
}

pub trait BuildFingerprintStore: Send + Sync {
    fn load(&self, config_dir: &Path) -> HashMap<String, String>;
    fn save(
        &self,
        config_dir: &Path,
        fingerprints: &HashMap<String, String>,
    ) -> Result<(), String>;
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
